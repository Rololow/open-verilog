# Philosophy

Open Verilog is a SystemVerilog superset with a safe core, the way Rust is a systems language with a safe core. The
two are meant to live together: a design moves from SystemVerilog to Open Verilog one file or one module at a time.

## Two zones

- **SystemVerilog is the zone the compiler does not vouch for.** A `.sv` file, or a module written in SystemVerilog
  inside a `.ov` project (`mixed`), is passed on as it is. `ovc lint` checks it, but nothing is guaranteed about it.
  This is the `unsafe` of Rust: legacy IP, vendor code and anything the compiler cannot prove stay here, and nothing
  is rewritten to make them fit.
- **Open Verilog is the zone the compiler checks.** The rules that catch real mistakes are errors by default: a
  signal's bits are not dropped without a `.wrap()`, a signal has one driver, a combinational signal is assigned on
  every path, a signal of one clock domain is not read in another without a synchronizer. These are the borrow
  rules of Rust: a design that breaks them is not compiled.

## Escapes are explicit and listed

Each place where the author asks the compiler to trust them is written in the source, and `ovc audit` lists it:

- `.wrap()`: a truncation that is meant (the bits kept are the bits intended).
- `async` on a port or a declaration, and `same_domain`: the clock crossing is not checked there.
- `// ovc: ignore <category>`: one finding silenced on its line.
- a lint level set below its default in `ovc.toml`.
- the SystemVerilog zone: a `.sv` file, or a `mixed` module. `ovc audit` does not list these yet.

An escape is allowed. It is not hidden: a reviewer can read the audit and see what the design does not promise.

## Migrating

- Preprocessor directives work in a `.ov` file: `` `define ``, `` `ifdef `` / `` `ifndef `` / `` `else `` / `` `endif ``
  and the macro uses. `` `include `` and the `-D` options are not in a `.ov` file yet.
- A value that a macro gives is better as a `config` or a `val`, where its type is checked.
- Keep a file in SystemVerilog until it is worth moving, and move the parts whose bits and clocks the compiler can check.
