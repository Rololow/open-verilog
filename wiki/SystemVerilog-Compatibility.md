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

- `cdc`, `handshake`, `reset`: the clock-domain, valid/ready and reset checks (the resets are the declared ones: `reset rst;`
  in `.ov`, or the annotation below in `.sv`);
- `sign`: a signed operand mixed with an unsigned one, and a signed operand extended with zeros;
- `sizing`: a sum or a product shifted right, evaluated on fewer bits than it needs;
- `width`, `carry`, `slice`: assignments and connections of the wrong width, lost carries, slices out of range;
- `syntax`: a select of bits of an expression, which SystemVerilog does not allow;
- `duplicate`: a typedef defined in two files;
- `latch`: a combinational signal not assigned on every path;
- `driver`: a signal driven from two places, an output that nothing drives;
- `connection`: an input without default that an instance leaves unconnected;
- `process`: `<=` in an `always_comb`, `=` on a signal another process reads in a clocked block.

### Declaring the clock and the reset of a `.sv` module

A plain `.sv` module has no `clock` or `reset` line, so the reset and clock-domain checks need a comment that declares
them. It is read by the lint only, and the source is not changed:

```systemverilog
module Plain (input wire clk, input wire rst_n, output reg [3:0] q);
  // ovc: clock clk
  // ovc: reset rst_n low
  always @(posedge clk) if (!rst_n) q <= 0; else q <= q + 1;
endmodule
```

The annotation belongs to the module above it. The name must be a port of the module, `low` and `async` are the same as in
`reset rst low async;`. A reset tested with the other polarity than the one declared (`if (rst)` for `rst_n low`) is
reported.

`break;` in a `for` loop is read: the iterations after the one that breaks do nothing, as in SystemVerilog. A `break` outside
a loop is an error. A `localparam` in a generate loop is a constant of each iteration.

Known limits:

- a directive in the middle of a line, such as `` `ifdef `` after code on the same line, is not handled: put the
  directive on its own line (the generated SystemVerilog of `ovc` does this in its assertions);
- the concurrent assertions (`label: assert property (...) else ...;`) are skipped: they are checks for the simulator, not
  part of the design;
- a typedef clash is reported on line 1 (the typedefs do not keep their line yet);
- the findings of an assignment are placed on the block that holds it (the statements do not keep a position yet);
- the messages are not rewritten for `.sv` files (the advice of `.ov` files still says `.wrap()`).
