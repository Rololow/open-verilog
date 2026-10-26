# Contributing

## Setup

You need a stable Rust toolchain. The compiler uses the standard library only; the language server adds `tower-lsp`
and `tokio`.

```
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## Rules

- The Python reference decides the behaviour. When the two differ on a construct the examples use, change the Rust
  output to match.
- Every new feature starts from an example in `examples/`, with its expected output in `expected/`.
- Repository text is in English: code, comments, docs and commit messages.

## Tests

- The golden tests (`tests/golden.rs`) compare the output for each example with `expected/`, byte for byte.
- The other test files cover diagnostics, the pipeline, declarations, manifests, struct views, the testbench, the probes
  and the library.
- Regression tests go next to the feature they cover.

## Continuous integration

Workflows are in `.github/workflows/`:

- `ci.yml` runs on every push and pull request. It builds, tests and runs clippy on Linux and Windows. The `docs` job
  builds the compiler and runs `scripts/check_wiki.py`: it checks that every link between pages of this wiki resolves,
  and that every `ov` block on a page compiles.
- `release.yml` runs on a tag `v*`. It tests and builds the binaries for Windows, Linux and macOS, builds the Windows
  installer, and publishes them with `SHA256SUMS`.
- `wiki.yml` runs when a page of `wiki/` changes on `main`. It copies the pages to the GitHub wiki.

## Releases

1. Bump `version` in `Cargo.toml`, and refresh `Cargo.lock` with a build.
2. Commit, then tag and push: `git tag -a vX.Y.Z -m "ovc X.Y.Z"` and `git push origin main vX.Y.Z`.
3. Check the `release` workflow. The VS Code extension downloads the latest release on its own.

## Wiki

The pages are the files in `wiki/`. Edit them there, in a pull request, like any other file. The `wiki` workflow copies
them to the GitHub wiki after the merge. Do not edit the wiki in the browser: the next sync overwrites the change.

The first time, the wiki must exist: enable it in the repository settings and create one page in the browser. The
workflow then pushes to the wiki repository, which GitHub creates with that first page.
