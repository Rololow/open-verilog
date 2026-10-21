//! ovc-lsp: the language server of Open Verilog, for any LSP client (the VS Code extension starts it).
//!
//! - diagnostics: the errors and warnings of the workspace's `.ov` files, compiled together as `ovc build` does
//!   (a project shares one namespace); the open buffers are compiled with their unsaved text. While typing, the
//!   compilation waits for a short pause; on save and on open it runs at once;
//! - go-to-definition: the declaration of the identifier under the cursor (module, enum, struct, bundle, config,
//!   port, signal, fn, task, val or const), in the file it is written in first, then in the other files. The
//!   declarations of each file are kept, and recomputed only when its text changes.
//!
//! Positions are the compiler's (1-based line and column); the protocol's are 0-based.

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ovc::ast::{File, Pos};
use ovc::lexer::{lex, Kind};
use ovc::{compile_units, Options};
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// How long the typing must pause before the workspace is compiled again.
const PAUSE: Duration = Duration::from_millis(600);

/// The declarations of a file, with the hash of the text they were read from.
type Declared = (u64, Vec<(String, Pos)>);

#[derive(Default)]
struct State {
    /// The workspace folders, as paths.
    roots: Vec<PathBuf>,
    /// The open buffers: their text is the one that is compiled (it may not be saved yet).
    open: HashMap<PathBuf, String>,
    /// The files that had diagnostics the last time: they get an empty list once they are clean.
    published: HashSet<Url>,
    /// The declarations of each file, by path, while its text does not change.
    declared: HashMap<PathBuf, Declared>,
    /// Counts the edits: a compilation scheduled by an edit runs only if no edit came after it.
    generation: u64,
}

struct Backend {
    client: Client,
    state: Arc<Mutex<State>>,
}

/// Runs compilation work on a thread with a large stack, as the CLI does: elaboration recurses through deep
/// expressions, and the default stack of a server thread overflows on real designs.
fn on_big_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(work)
        .expect("start the compiler thread")
        .join()
        .expect("the compiler thread")
}

/// The `.ov` files under the workspace folders (the build folders and the hidden ones are skipped).
fn ov_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !name.starts_with('.') && !matches!(name.as_str(), "target" | "out" | "node_modules") {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "ov") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for root in roots {
        walk(root, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The text of a file: its open buffer, else what is on disk.
fn text_of(state: &State, path: &Path) -> Option<String> {
    state.open.get(path).cloned().or_else(|| std::fs::read_to_string(path).ok())
}

/// Every `.ov` unit of the workspace, with the text it has now: (path, text).
fn units(state: &State) -> Vec<(PathBuf, String)> {
    let mut paths = ov_files(&state.roots);
    for path in state.open.keys() {
        if path.extension().is_some_and(|e| e == "ov") && !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    paths.sort();
    paths.into_iter().filter_map(|p| text_of(state, &p).map(|t| (p, t))).collect()
}

fn range_at(line: usize, col: usize, len: usize) -> Range {
    let start = Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32);
    let end = Position::new(start.line, start.character + len.max(1) as u32);
    Range::new(start, end)
}

/// `path:line:col: warning: message`, as the compiler prints its warnings.
fn parse_warning(text: &str) -> Option<(String, usize, usize, String)> {
    let (head, message) = text.split_once(": warning: ")?;
    let mut parts = head.rsplitn(3, ':');
    let col = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let path = parts.next()?.to_string();
    Some((path, line, col, message.to_string()))
}

/// The diagnostics of a compilation, per file.
fn diagnostics(units: &[(PathBuf, String)]) -> HashMap<String, Vec<Diagnostic>> {
    let named: Vec<(String, String)> = units.iter().map(|(p, t)| (p.to_string_lossy().to_string(), t.clone())).collect();
    let mut out: HashMap<String, Vec<Diagnostic>> = HashMap::new();
    match compile_units(&named, &Options::default()) {
        Err(e) => {
            let diag = Diagnostic {
                range: range_at(e.line, e.col, 1),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("ovc".into()),
                message: e.message.clone(),
                ..Default::default()
            };
            out.entry(e.file.clone()).or_default().push(diag);
        }
        Ok(compiled) => {
            for warning in &compiled.warnings {
                if let Some((path, line, col, message)) = parse_warning(warning) {
                    let diag = Diagnostic {
                        range: range_at(line, col, 1),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("ovc".into()),
                        message,
                        ..Default::default()
                    };
                    out.entry(path).or_default().push(diag);
                }
            }
        }
    }
    out
}

/// The identifier under a position (0-based line and character), if there is one.
fn ident_at(text: &str, line: u32, character: u32) -> Option<String> {
    let tokens = lex(text, "").ok()?;
    let (line, col) = (line as usize + 1, character as usize + 1);
    tokens
        .into_iter()
        .find(|t| t.kind == Kind::Ident && t.line == line && t.col <= col && col < t.col + t.text.chars().count())
        .map(|t| t.text)
}

/// The declarations of a file: the name, and where it is written.
fn declarations(file: &File) -> Vec<(String, Pos)> {
    let mut out = Vec::new();
    for m in &file.modules {
        out.push((m.name.clone(), m.pos));
        for p in &m.ports {
            out.push((p.name.clone(), p.pos));
        }
        for item in &m.items {
            if let ovc::ast::ItemKind::Decl(d) = &item.kind {
                for x in &d.decls {
                    out.push((x.name.clone(), item.pos));
                }
            }
        }
    }
    out.extend(file.enums.iter().map(|e| (e.name.clone(), e.pos)));
    out.extend(file.structs.iter().map(|s| (s.name.clone(), s.pos)));
    out.extend(file.bundles.iter().map(|b| (b.name.clone(), b.pos)));
    out.extend(file.configs.iter().map(|c| (c.name.clone(), c.pos)));
    out
}

/// The functions, tasks, vals and constants of a file, which the AST does not keep with their position: found by
/// their keyword followed by the name (`fn pmux(...)` names `pmux` at the position of `fn`).
fn keyword_declarations(text: &str) -> Vec<(String, Pos)> {
    let Ok(tokens) = lex(text, "") else { return Vec::new() };
    tokens
        .windows(2)
        .filter(|pair| pair[0].kind == Kind::Ident && matches!(pair[0].text.as_str(), "fn" | "task" | "val" | "const"))
        .filter(|pair| pair[1].kind == Kind::Ident)
        .map(|pair| (pair[1].text.clone(), (pair[0].line, pair[0].col)))
        .collect()
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The declarations of one file: the AST's and the keyword-named ones.
fn declarations_of(path: &Path, text: &str) -> Vec<(String, Pos)> {
    let mut found = ovc::parser::parse_file(text, &path.to_string_lossy()).map(|f| declarations(&f)).unwrap_or_default();
    found.extend(keyword_declarations(text));
    found
}

/// The declaration of `name`: in the file of the use first, then in the others. `declared` holds every file's.
fn find_definition(name: &str, here: &Path, declared: &[(PathBuf, Vec<(String, Pos)>)]) -> Option<(PathBuf, Pos)> {
    let in_file = |path: &PathBuf| {
        declared
            .iter()
            .filter(|(p, _)| p == path)
            .flat_map(|(p, found)| found.iter().map(move |(n, pos)| (p.clone(), n, *pos)))
            .find(|(_, n, _)| n.as_str() == name)
            .map(|(p, _, pos)| (p, pos))
    };
    in_file(&here.to_path_buf()).or_else(|| declared.iter().find_map(|(p, _)| in_file(p)))
}

fn path_of(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

/// Compiles the workspace and publishes its diagnostics (the files that are clean get an empty list).
async fn publish(client: &Client, state: &Mutex<State>) {
    let units = {
        let state = state.lock().await;
        units(&state)
    };
    let per_file = tokio::task::spawn_blocking(move || on_big_stack(move || diagnostics(&units))).await.unwrap_or_default();
    let mut now = HashSet::new();
    for (file, diags) in per_file {
        let Ok(uri) = Url::from_file_path(&file) else { continue };
        now.insert(uri.clone());
        client.publish_diagnostics(uri, diags, None).await;
    }
    let previous: Vec<Url> = {
        let mut state = state.lock().await;
        let old = state.published.iter().filter(|u| !now.contains(u)).cloned().collect();
        state.published = now;
        old
    };
    for uri in previous {
        client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

impl Backend {
    /// Compiles now: on open, on save, on close.
    async fn check_now(&self) {
        self.state.lock().await.generation += 1;
        publish(&self.client, &self.state).await;
    }

    /// Compiles after a pause in the typing: an edit that comes in the meantime cancels this one.
    async fn check_after_pause(&self) {
        let mine = {
            let mut state = self.state.lock().await;
            state.generation += 1;
            state.generation
        };
        let client = self.client.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            tokio::time::sleep(PAUSE).await;
            if state.lock().await.generation == mine {
                publish(&client, &state).await;
            }
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut state = self.state.lock().await;
        if let Some(folders) = params.workspace_folders {
            state.roots = folders.iter().filter_map(|f| path_of(&f.uri)).collect();
        } else if let Some(root) = params.root_uri.as_ref().and_then(path_of) {
            state.roots = vec![root];
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo { name: "ovc-lsp".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.check_now().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if let Some(path) = path_of(&params.text_document.uri) {
            self.state.lock().await.open.insert(path, params.text_document.text);
        }
        self.check_now().await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let (Some(path), Some(change)) = (path_of(&params.text_document.uri), params.content_changes.into_iter().last()) {
            self.state.lock().await.open.insert(path, change.text);
        }
        self.check_after_pause().await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let (Some(path), Some(text)) = (path_of(&params.text_document.uri), params.text) {
            self.state.lock().await.open.insert(path, text);
        }
        self.check_now().await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Some(path) = path_of(&params.text_document.uri) {
            self.state.lock().await.open.remove(&path);
        }
        self.check_now().await;
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(path) = path_of(&uri) else { return Ok(None) };
        let (units, cache, text) = {
            let state = self.state.lock().await;
            (units(&state), state.declared.clone(), text_of(&state, &path))
        };
        let Some(text) = text else { return Ok(None) };
        let Some(name) = ident_at(&text, position.line, position.character) else { return Ok(None) };

        // the declarations of every file: from the cache while its text is the same, else read again
        let work = move || {
            let mut fresh = Vec::new();
            let declared: Vec<(PathBuf, Vec<(String, Pos)>)> = units
                .iter()
                .map(|(p, t)| {
                    let hash = text_hash(t);
                    match cache.get(p) {
                        Some((h, found)) if *h == hash => (p.clone(), found.clone()),
                        _ => {
                            let found = declarations_of(p, t);
                            fresh.push((p.clone(), (hash, found.clone())));
                            (p.clone(), found)
                        }
                    }
                })
                .collect();
            let found = find_definition(&name, &path, &declared);
            (found, fresh)
        };
        let (found, fresh) = tokio::task::spawn_blocking(move || on_big_stack(work)).await.unwrap_or((None, Vec::new()));
        self.state.lock().await.declared.extend(fresh);

        let Some((file, (line, col))) = found else { return Ok(None) };
        let Ok(target) = Url::from_file_path(&file) else { return Ok(None) };
        let at = Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32);
        Ok(Some(GotoDefinitionResponse::Scalar(Location::new(target, Range::new(at, at)))))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend { client, state: Arc::new(Mutex::new(State::default())) });
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_keep_their_position_even_with_a_drive_letter() {
        let (path, line, col, message) = parse_warning("C:/proj/a.ov:12:5: warning: 'x' is unused").unwrap();
        assert_eq!((path.as_str(), line, col, message.as_str()), ("C:/proj/a.ov", 12, 5, "'x' is unused"));
    }

    #[test]
    fn the_identifier_under_the_cursor() {
        let src = "module Counter(input clk) {\n  logic [7:0] q;\n}\n";
        assert_eq!(ident_at(src, 0, 8).as_deref(), Some("Counter"));
        assert_eq!(ident_at(src, 1, 14).as_deref(), Some("q"));
        assert_eq!(ident_at(src, 0, 6), None);
    }

    #[test]
    fn a_function_is_found_by_its_keyword() {
        let src = "module M(input logic a) {}\nfn pmux(pairs, dflt) { return dflt; }\n";
        assert_eq!(keyword_declarations(src), vec![("pmux".to_string(), (2, 1))]);
    }

    #[test]
    fn a_module_is_found_in_another_file() {
        let a = (PathBuf::from("/w/a.ov"), vec![("Top".to_string(), (1, 1))]);
        let b = (PathBuf::from("/w/b.ov"), vec![("Adder".to_string(), (1, 1))]);
        let (file, pos) = find_definition("Adder", &PathBuf::from("/w/a.ov"), &[a, b]).unwrap();
        assert_eq!((file, pos), (PathBuf::from("/w/b.ov"), (1, 1)));
    }

    #[test]
    fn the_same_text_has_the_same_hash_and_an_edit_changes_it() {
        assert_eq!(text_hash("module A {}"), text_hash("module A {}"));
        assert_ne!(text_hash("module A {}"), text_hash("module B {}"));
    }
}
