//! ovc-lsp: the language server of Open Verilog, for any LSP client (the VS Code extension starts it).
//!
//! - diagnostics: the errors and warnings of the workspace's `.ov` files, compiled together as `ovc build` does
//!   (a project shares one namespace); the open buffers are compiled with their unsaved text. While typing, the
//!   compilation waits for a short pause; on save and on open it runs at once;
//! - go-to-definition: the declaration of the identifier under the cursor (module, enum, struct, bundle, config,
//!   port, signal, fn, task, val or const), in the file it is written in first, then in the other files. The
//!   declarations of each file are kept, and recomputed only when its text changes;
//! - hover: the declaration line of the identifier under the cursor, with its file and line (the same lookup as the
//!   go-to-definition).
//!
//! - the lint levels of the workspace's `ovc.toml` (its `[lints]` table) apply to the diagnostics, as they do to the CLI;
//! - completion: the names visible in the module of the cursor, the names of the workspace, and the keywords;
//! - document and workspace symbols: the modules, ports, signals, enums, structs, bundles, configs, functions, tasks,
//!   vals and constants.
//!
//! Positions are the compiler's (1-based line and column); the protocol's are 0-based.

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ovc::ast::{File, ItemKind, Pos};
use ovc::diag::Levels;
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
fn diagnostics(units: &[(PathBuf, String)], levels: &Levels) -> HashMap<String, Vec<Diagnostic>> {
    let named: Vec<(String, String)> = units.iter().map(|(p, t)| (p.to_string_lossy().to_string(), t.clone())).collect();
    let mut out: HashMap<String, Vec<Diagnostic>> = HashMap::new();
    let options = Options { levels: levels.clone(), ..Options::default() };
    match compile_units(&named, &options) {
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

/// The declaration of `name`: in the file of the use first, then in the others (in path order). `declared` holds every
/// file's.
fn find_definition(name: &str, here: &Path, declared: &HashMap<PathBuf, Declared>) -> Option<(PathBuf, Pos)> {
    let in_file = |path: &Path| {
        let (_, found) = declared.get(path)?;
        found.iter().find(|(n, _)| n.as_str() == name).map(|(_, pos)| (path.to_path_buf(), *pos))
    };
    let mut paths: Vec<&PathBuf> = declared.keys().collect();
    paths.sort();
    in_file(here).or_else(|| paths.into_iter().find_map(|p| in_file(p)))
}

fn path_of(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

/// Compiles the workspace and publishes its diagnostics (the files that are clean get an empty list).
/// The declarations are kept here as well, so that go-to-definition and hover do not read the workspace again.
async fn publish(client: &Client, state: &Mutex<State>) {
    let (units, roots, cache) = {
        let state = state.lock().await;
        (units(&state), state.roots.clone(), state.declared.clone())
    };
    let levels = match levels_of(&roots) {
        Ok(levels) => levels,
        Err(message) => {
            client.log_message(MessageType::ERROR, message).await;
            Levels::default()
        }
    };
    let paths: HashSet<PathBuf> = units.iter().map(|(p, _)| p.clone()).collect();
    // the declarations of the files whose text changed since they were read
    let (per_file, fresh) = tokio::task::spawn_blocking(move || {
        on_big_stack(move || {
            let per_file = diagnostics(&units, &levels);
            let mut fresh = Vec::new();
            for (p, t) in &units {
                let hash = text_hash(t);
                if !matches!(cache.get(p), Some((h, _)) if *h == hash) {
                    fresh.push((p.clone(), (hash, declarations_of(p, t))));
                }
            }
            (per_file, fresh)
        })
    })
    .await
    .unwrap_or_default();
    {
        let mut state = state.lock().await;
        state.declared.retain(|p, _| paths.contains(p));
        state.declared.extend(fresh);
    }
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
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions { trigger_characters: Some(vec![".".into()]), ..Default::default() }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
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
        let position = params.text_document_position_params;
        let Some((_, file, (line, col), _)) = self.resolve(&position.text_document.uri, position.position).await else {
            return Ok(None);
        };
        let Ok(target) = Url::from_file_path(&file) else { return Ok(None) };
        let at = Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32);
        Ok(Some(GotoDefinitionResponse::Scalar(Location::new(target, Range::new(at, at)))))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        let Some(path) = path_of(&position.text_document.uri) else { return Ok(None) };
        let (units, text) = {
            let state = self.state.lock().await;
            (units(&state), text_of(&state, &path))
        };
        let line = position.position.line as usize + 1;
        let work = move || {
            let mut names: Vec<(String, CompletionItemKind)> = Vec::new();
            if let Some(text) = &text {
                names.extend(names_in_scope(&path, text, line).into_iter().map(|(n, k)| (n, completion_kind(k))));
            }
            for (p, t) in &units {
                names.extend(symbols_of(p, t).into_iter().map(|(n, _, k)| (n, completion_kind(k))));
            }
            names.extend(keywords().map(|k| (k.to_string(), CompletionItemKind::KEYWORD)));
            names
        };
        let names = tokio::task::spawn_blocking(move || on_big_stack(work)).await.unwrap_or_default();
        // the names of the module come first: a name that is both in the module and in the workspace keeps its first entry
        let mut seen = HashSet::new();
        let items = names
            .into_iter()
            .filter(|(name, _)| seen.insert(name.clone()))
            .map(|(label, kind)| CompletionItem { label, kind: Some(kind), ..Default::default() })
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(&self, params: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(path) = path_of(&uri) else { return Ok(None) };
        let Some(text) = text_of(&*self.state.lock().await, &path) else { return Ok(None) };
        let symbols = tokio::task::spawn_blocking(move || on_big_stack(move || symbols_of(&path, &text))).await.unwrap_or_default();
        let found = symbols
            .into_iter()
            .map(|(name, pos, kind)| {
                let range = range_at(pos.0, pos.1, name.chars().count());
                symbol_info(name, kind, Location::new(uri.clone(), range))
            })
            .collect();
        Ok(Some(DocumentSymbolResponse::Flat(found)))
    }

    async fn symbol(&self, params: WorkspaceSymbolParams) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();
        let units = {
            let state = self.state.lock().await;
            units(&state)
        };
        let found = tokio::task::spawn_blocking(move || {
            on_big_stack(move || {
                let mut out = Vec::new();
                for (path, text) in &units {
                    let Ok(uri) = Url::from_file_path(path) else { continue };
                    for (name, pos, kind) in symbols_of(path, text) {
                        if name.to_lowercase().contains(&query) {
                            let range = range_at(pos.0, pos.1, name.chars().count());
                            out.push(symbol_info(name, kind, Location::new(uri.clone(), range)));
                        }
                    }
                }
                out.truncate(500);
                out
            })
        })
        .await
        .unwrap_or_default();
        Ok(Some(found))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params;
        let Some(path) = path_of(&position.text_document.uri) else { return Ok(None) };
        let (line0, character) = (position.position.line, position.position.character);
        let text = {
            let state = self.state.lock().await;
            text_of(&state, &path)
        };
        let Some(text) = text else { return Ok(None) };
        let Some(name) = ident_at(&text, line0, character) else { return Ok(None) };
        let dotted = follows_dot(&text, line0, character);
        let markdown = |value: String| {
            Ok(Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range: None }))
        };

        // a method or function of the language: what it does (a name after a dot is a method, `tie_off(b)` a function)
        if let Some((_, signature, doc)) = BUILTINS.iter().find(|(n, s, _)| *n == name && s.starts_with('.') == dotted) {
            return markdown(format!("```ovc\n{signature}\n```\n{doc}"));
        }
        // a declaration of the workspace: its line, as the compiler sees it
        if let Some((name, file, (line, _), file_text)) = self.resolve(&position.text_document.uri, position.position).await {
            if let Some(decl) = declaration_line(&file_text, line) {
                let file_name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                return markdown(format!("```ovc\n{decl}\n```\n`{name}`, declared in `{file_name}` line {line}"));
            }
        }
        // an entry of the standard library, when the workspace does not declare the name (the workspace's wins)
        if let Some((decl, doc)) = stdlib_entry(&name) {
            let doc = if doc.is_empty() { String::new() } else { format!("\n\n{doc}") };
            return markdown(format!("```ovc\n{decl}\n```\nStandard library{doc}"));
        }
        Ok(None)
    }
}

impl Backend {
    /// The identifier under a position, and where it is declared: (name, file, position, text of that file).
    async fn resolve(&self, uri: &Url, position: Position) -> Option<(String, PathBuf, Pos, String)> {
        let path = path_of(uri)?;
        // the declarations of the workspace are those of the last compilation (see `publish`): no file is read here
        let (name, found) = {
            let state = self.state.lock().await;
            let name = ident_at(&text_of(&state, &path)?, position.line, position.character)?;
            let found = find_definition(&name, &path, &state.declared);
            (name, found)
        };
        let (file, pos) = found?;
        let file_text = text_of(&*self.state.lock().await, &file)?;
        Some((name, file, pos, file_text))
    }
}

/// The standard library's source: the doc of an entry is the comment above its declaration.
const STDLIB: &str = include_str!("../stdlib/stdlib.ov");

/// The methods and functions of the language, for the hover: (name, signature, what it does). A signature that starts
/// with a dot is a method (it is explained after a dot), the others are functions.
const BUILTINS: &[(&str, &str, &str)] = &[
    ("wrap", ".wrap()", "marks a truncation as intended: the arithmetic under it is computed exactly, then truncated to the width of the target. Without it, a value wider than its target is an error (the `width` and `carry` checks, `deny` by default)."),
    ("delay", ".delay(n)", "the value delayed by `n` clock cycles: a chain of `n` registers (`delay(0)` is the value itself). The module needs its `clock`."),
    ("reverse", ".reverse()", "the bits in reverse order."),
    ("fill", ".fill(n)", "the value repeated `n` times, as `{n{value}}`."),
    ("zext", ".zext(w)", "zero-extension to `w` bits."),
    ("sext", ".sext(w)", "sign-extension to `w` bits."),
    ("msb", ".msb()", "the first (most significant) bit."),
    ("lsb", ".lsb()", "the last (least significant) bit."),
    ("list", ".list()", "turns an array signal into a list known at compile time: `v[0]`, `v[1]`, ... A list has `map`, `filter`, `reduce` and `tree_reduce`."),
    ("map", ".map(f)", "applies `f` to each element of a list, at compile time."),
    ("filter", ".filter(f)", "keeps the elements of a list for which `f` holds, at compile time."),
    ("reduce", ".reduce(op)", "combines the elements of a list as a chain: `((a0 op a1) op a2) ...`."),
    ("tree_reduce", ".tree_reduce(op)", "combines the elements of a list as a balanced tree instead of a chain (for four inputs: `in[0] + in[1] + (in[2] + in[3])`). The tree is built at elaboration."),
    ("as", ".as(S)", "a view of the bits of the value as the struct or instruction format `S`; its fields are read by name. The value must be as wide as `S`."),
    ("tie_off", "tie_off(b)", "gives the idle value to a master bundle that is not driven. A module output that nothing drives is an error: drive it, or tie it off."),
];

/// Whether a line declares `name` as a module, bundle, struct, enum or function.
fn declares(line: &str, name: &str) -> bool {
    let mut words = line.trim_start().split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty());
    matches!(words.next(), Some("module" | "bundle" | "struct" | "enum" | "fn")) && words.next() == Some(name)
}

/// A standard-library entry: its declaration line, and the comment above it.
fn stdlib_entry(name: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = STDLIB.lines().collect();
    let at = lines.iter().position(|l| declares(l, name))?;
    let mut doc: Vec<&str> = lines[..at]
        .iter()
        .rev()
        .take_while(|l| l.starts_with("//"))
        .map(|l| l.trim_start_matches('/').trim())
        .collect();
    doc.reverse();
    Some((lines[at].trim().trim_end_matches('{').trim_end().to_string(), doc.join(" ")))
}

/// Whether the identifier at a position follows a `.` (a method, or a field).
fn follows_dot(text: &str, line: u32, character: u32) -> bool {
    let Ok(tokens) = lex(text, "") else { return false };
    let (line, col) = (line as usize + 1, character as usize + 1);
    tokens.windows(2).any(|p| {
        p[0].text == "."
            && p[1].kind == Kind::Ident
            && p[1].line == line
            && p[1].col <= col
            && col < p[1].col + p[1].text.chars().count()
    })
}

/// The lint levels of the workspace: the `[lints]` of the `ovc.toml` of the first folder that has one.
fn levels_of(roots: &[PathBuf]) -> std::result::Result<Levels, String> {
    for root in roots {
        if let Ok(text) = std::fs::read_to_string(root.join("ovc.toml")) {
            return Levels::from_manifest(&text);
        }
    }
    Ok(Levels::default())
}

/// The words of the language that complete at any position.
/// The words that complete at any position: the Open Verilog words and the SystemVerilog ones (see `keywords`).
fn keywords() -> impl Iterator<Item = &'static str> {
    ovc::keywords::OV_WORDS.iter().chain(ovc::keywords::SV_WORDS).copied()
}

/// The symbols a file declares, with their kind and position: modules, ports, signals, enums, structs, bundles, configs,
/// functions, tasks, vals and constants.
fn symbols_of(path: &Path, text: &str) -> Vec<(String, Pos, SymbolKind)> {
    let mut out = Vec::new();
    if let Ok(file) = ovc::parser::parse_file(text, &path.to_string_lossy()) {
        for m in &file.modules {
            out.push((m.name.clone(), m.pos, SymbolKind::MODULE));
            out.extend(m.ports.iter().map(|p| (p.name.clone(), p.pos, SymbolKind::VARIABLE)));
            for item in &m.items {
                if let ItemKind::Decl(d) = &item.kind {
                    let kind = if d.is_const { SymbolKind::CONSTANT } else { SymbolKind::VARIABLE };
                    out.extend(d.decls.iter().map(|x| (x.name.clone(), item.pos, kind)));
                }
            }
        }
        out.extend(file.enums.iter().map(|e| (e.name.clone(), e.pos, SymbolKind::ENUM)));
        out.extend(file.structs.iter().map(|s| (s.name.clone(), s.pos, SymbolKind::STRUCT)));
        out.extend(file.bundles.iter().map(|b| (b.name.clone(), b.pos, SymbolKind::STRUCT)));
        out.extend(file.configs.iter().map(|c| (c.name.clone(), c.pos, SymbolKind::CLASS)));
    }
    out.extend(keyword_symbols(text));
    out
}

/// The functions, tasks, vals and constants of a file, found by their keyword (see `keyword_declarations`), with kind.
fn keyword_symbols(text: &str) -> Vec<(String, Pos, SymbolKind)> {
    let Ok(tokens) = lex(text, "") else { return Vec::new() };
    tokens
        .windows(2)
        .filter(|pair| pair[0].kind == Kind::Ident && pair[1].kind == Kind::Ident)
        .filter_map(|pair| {
            let kind = match pair[0].text.as_str() {
                "fn" | "task" => SymbolKind::FUNCTION,
                "val" | "const" => SymbolKind::CONSTANT,
                _ => return None,
            };
            Some((pair[1].text.clone(), (pair[0].line, pair[0].col), kind))
        })
        .collect()
}

/// The names visible at a line (1-based): the ports and declarations of the module that starts last before it. Empty
/// when the file does not parse, which is the case while it is being typed.
fn names_in_scope(path: &Path, text: &str, line: usize) -> Vec<(String, SymbolKind)> {
    let Ok(file) = ovc::parser::parse_file(text, &path.to_string_lossy()) else { return Vec::new() };
    let Some(m) = file.modules.iter().rfind(|m| m.pos.0 <= line) else { return Vec::new() };
    let mut out: Vec<(String, SymbolKind)> = m.ports.iter().map(|p| (p.name.clone(), SymbolKind::VARIABLE)).collect();
    for item in &m.items {
        if let ItemKind::Decl(d) = &item.kind {
            out.extend(d.decls.iter().map(|x| (x.name.clone(), SymbolKind::VARIABLE)));
        }
    }
    out
}

fn completion_kind(kind: SymbolKind) -> CompletionItemKind {
    match kind {
        SymbolKind::MODULE => CompletionItemKind::MODULE,
        SymbolKind::ENUM => CompletionItemKind::ENUM,
        SymbolKind::STRUCT => CompletionItemKind::STRUCT,
        SymbolKind::CLASS => CompletionItemKind::CLASS,
        SymbolKind::FUNCTION => CompletionItemKind::FUNCTION,
        SymbolKind::CONSTANT => CompletionItemKind::CONSTANT,
        _ => CompletionItemKind::VARIABLE,
    }
}

#[allow(deprecated)]
fn symbol_info(name: String, kind: SymbolKind, location: Location) -> SymbolInformation {
    SymbolInformation { name, kind, tags: None, deprecated: None, location, container_name: None }
}

/// The text of a line of a file (1-based), trimmed, if it is not empty.
fn declaration_line(text: &str, line: usize) -> Option<String> {
    text.lines().nth(line.checked_sub(1)?).map(str::trim).filter(|l| !l.is_empty()).map(str::to_string)
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
        let declared: HashMap<PathBuf, Declared> = HashMap::from([
            (PathBuf::from("/w/a.ov"), (0, vec![("Top".to_string(), (1, 1))])),
            (PathBuf::from("/w/b.ov"), (0, vec![("Adder".to_string(), (1, 1))])),
        ]);
        let (file, pos) = find_definition("Adder", &PathBuf::from("/w/a.ov"), &declared).unwrap();
        assert_eq!((file, pos), (PathBuf::from("/w/b.ov"), (1, 1)));
    }

    #[test]
    fn the_declaration_line_is_the_trimmed_line_of_the_position() {
        assert_eq!(declaration_line("a
  logic [7:0] q;
", 2).as_deref(), Some("logic [7:0] q;"));
        assert_eq!(declaration_line("a

", 2), None);
        assert_eq!(declaration_line("a", 3), None);
        assert_eq!(declaration_line("a", 0), None);
    }

    #[test]
    fn the_symbols_of_a_file_have_their_kind() {
        let src = "module Top(input logic a) {\n  logic [7:0] q;\n  const K = 4;\n}\nfn f(x) { return x; }\n";
        let symbols = symbols_of(&PathBuf::from("t.ov"), src);
        let kind = |name: &str| symbols.iter().find(|(n, _, _)| n == name).map(|(_, _, k)| *k);
        assert_eq!(kind("Top"), Some(SymbolKind::MODULE));
        assert_eq!(kind("a"), Some(SymbolKind::VARIABLE));
        assert_eq!(kind("q"), Some(SymbolKind::VARIABLE));
        assert_eq!(kind("K"), Some(SymbolKind::CONSTANT));
        assert_eq!(kind("f"), Some(SymbolKind::FUNCTION));
    }

    #[test]
    fn the_names_in_scope_are_those_of_the_module_of_the_line() {
        let src = "module A(input logic x) {}\nmodule B(input logic y) {\n  logic z;\n}\n";
        let names: Vec<String> = names_in_scope(&PathBuf::from("t.ov"), src, 3).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["y".to_string(), "z".to_string()]);
    }

    #[test]
    fn the_lint_levels_of_the_manifest_reach_the_diagnostics() {
        let src = "module N(input logic [7:0] a, output logic [3:0] y) {\n  assign y = a;\n}\n";
        let units = vec![(PathBuf::from("/w/n.ov"), src.to_string())];
        let has_error = |levels: &Levels| {
            diagnostics(&units, levels).values().flatten().any(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        };
        assert!(has_error(&Levels::default()), "a width finding is an error by default");
        let mut warn = Levels::default();
        warn.set(ovc::diag::Category::Width, ovc::diag::Level::Warn);
        assert!(!has_error(&warn));
    }

    #[test]
    fn the_levels_are_read_from_the_manifest_of_the_first_folder_that_has_one() {
        let dir = std::env::temp_dir().join("ovc_lsp_levels_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ovc.toml"), "[lints]\nwidth = \"warn\"\n").unwrap();
        let levels = levels_of(std::slice::from_ref(&dir)).unwrap();
        assert_eq!(levels.get(ovc::diag::Category::Width), ovc::diag::Level::Warn);
        assert_eq!(levels.get(ovc::diag::Category::Carry), ovc::diag::Level::Deny);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_standard_library_entry_has_the_comment_above_it_as_its_doc() {
        let (decl, doc) = stdlib_entry("Sync").unwrap();
        assert!(decl.starts_with("module Sync<"), "{decl}");
        assert!(doc.contains("Two-flop synchronizer"), "{doc}");
        assert!(stdlib_entry("NoSuchEntry").is_none());
    }

    #[test]
    fn a_name_after_a_dot_is_a_method() {
        let src = "module M(input logic [7:0] a, output logic [7:0] y) {\n  assign y = (a + 1).wrap();\n}\n";
        assert!(follows_dot(src, 1, 21), "the 'w' of wrap follows the dot");
        assert!(!follows_dot(src, 1, 14), "the 'a' is not after a dot");
        assert!(BUILTINS.iter().any(|(n, s, _)| *n == "wrap" && s.starts_with('.')));
    }

    #[test]
    fn the_same_text_has_the_same_hash_and_an_edit_changes_it() {
        assert_eq!(text_hash("module A {}"), text_hash("module A {}"));
        assert_ne!(text_hash("module A {}"), text_hash("module B {}"));
    }
}
