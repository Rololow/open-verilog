# SystemVerilog in `.ov` files, and lint

An `.ov` file is a SystemVerilog file with extra constructs. Most plain SystemVerilog is accepted as it is, so a
project can move to `.ov` one file at a time. This page lists what the Rust compiler accepts, what it does not yet, and
the state of `lint`, the check of plain `.sv` files.

## What is accepted

Checked with `ovc` on each construct, in a `.ov` file:

| Construct | Status |
| --- | --- |
| `module` with `#(parameter ...)` ports, `always @(posedge clk)`, `begin ... end` | accepted |
| `generate` / `for (genvar i ...)` / `begin : name` | accepted |
| `initial` blocks with `$display`, `$clog2`, other `$` system functions | accepted |
| `typedef logic [7:0] byte_t;` used as a port type | accepted |
| Packed and unpacked arrays (`logic [7:0] a [4]`), `always_ff`, `always_comb`, `case` | accepted |

A plain `.sv` file goes through the same parser, so the generated SystemVerilog of any `.ov` file is also valid input.

## What is not accepted yet

| Construct | Error today |
| --- | --- |
| `function automatic ... endfunction` in a module | `'function' is not supported yet` |
| `` `define `` / `` `ifdef `` / `` `include `` (the preprocessor) | `unexpected character '`'` |

For a function, write the logic with the `.ov` form `fn name(...) -> type { return ...; }`. For a macro, use a
`config` or a `val` instead (see [Language tour](Language-Tour)). The preprocessor is part of the lint work below.

## Lint

`ovc lint` checks plain `.sv` and `.v` files without rewriting them. It preprocesses the files in order as one unit
(macros carry over), parses them, then checks them together:

```
ovc lint rtl/ +incdir+rtl/inc -DFAST=1 --only cdc,handshake --werror
ovc lint -f files.f --top core
```

Each finding gives its category, its file and line (the line as written, not the preprocessed one). A line can silence
a category: `// ovc: ignore cdc` (or `all`). Files that cannot be read, preprocessed or parsed are reported as not
analyzed, and the lint exits with 1.

The categories that exist today:

- `cdc`, `handshake`, `reset`: the clock-domain, valid/ready and reset checks;
- `latch`: a combinational signal not assigned on every path;
- `driver`: a signal driven from two places, an output that nothing drives;
- `connection`: an input without default that an instance leaves unconnected;
- `process`: `<=` in an `always_comb`, `=` on a signal another process reads in a clocked block.

Known limits:

- a directive in the middle of a line, such as `` `ifdef `` after code on the same line, is not handled: put the
  directive on its own line (the generated SystemVerilog of `ovc` has some, inside its assertions);
- the width checks (`width`, `carry`, `sizing`, `slice`, `sign`) are not in the Rust lint yet: use the Python lint for them;
- the findings of an assignment are placed on the block that holds it (the statements do not keep a position yet);
- the messages are not rewritten for `.sv` files (the advice of `.ov` files still says `.wrap()`).
