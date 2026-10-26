# Installation

## Release binaries

Each release is published on the GitHub releases page. Every release has:

- `ovc-windows-x64.exe`, `ovc-linux-x64`, `ovc-macos-arm64`, `ovc-macos-x64`: the compiler;
- `ovc-lsp-*`: the language server, with the same suffixes;
- `ovc-setup-<version>.exe`: a Windows installer, per user, which adds `ovc` to the `PATH`;
- `SHA256SUMS`: the checksums of all the files above.

Check a download against `SHA256SUMS` before running it.

## VS Code

The extension `vscode-open-verilog` gives syntax highlighting, diagnostics and go to definition. It downloads the
language server and the compiler on first use, and checks them against `SHA256SUMS`. Run the command
*Open Verilog: Install the compiler* to do it by hand.

## From source

You need a Rust toolchain (stable).

```
cargo build --release
```

The binaries are `target/release/ovc` and `target/release/ovc-lsp`. Run the tests before using them:

```
cargo test
```

See [Command line](Command-Line) for the options.
