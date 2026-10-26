# Reset and quality of life

Most of the code a module needs is the same in every module: a clock, a reset, a register that restarts at a value, a
signal whose width is the sum of its operands. Open Verilog states these once.

```ov
// `clk` and `rst` are detected; `reg x = init` is reset to init by `always_ff { }` (no @).
module Counter2<int W = 8>(input clk, input rst, input en, output logic [W-1:0] q) {
  reg [W-1:0] cnt = 0;
  always_ff {
    if (en) { cnt <= cnt + 1; }
  }
  assign q = cnt;
}

// explicitly declared active-low asynchronous reset
module Flag(input clk, input rstn, input set, input clr, output logic f) {
  reset rstn low async;
  reg r = 0;
  always_ff {
    if (set) { r <= 1; } else if (clr) { r <= 0; }
  }
  assign f = r;
}

// hardware `val` = named signal whose width is inferred losslessly (+: max+1, *: sum).
module Mac3(input logic [7:0] a, input logic [7:0] b, input logic [7:0] c, output logic [16:0] y) {
  val p = a * b;          // 16 bits
  val s = p + c;          // 17 bits
  assign y = s;
}

// val inside a pipeline stage: it is carried automatically like any other signal.
pipeline Mac2(
  input clk, input rst, input logic valid_in,
  input logic [7:0] a, input logic [7:0] b, input logic [7:0] c,
  output logic [15:0] y, output logic valid_out
) {
  stage mul { val p = a * b; }
  stage add { val s = p + add.c; }
  stage out { assign y = s.wrap(); }
}
```

## Clock and reset

- The module's clock and reset are found from their names (`clk`, `rst`). `always_ff { }` without an event is clocked by
  the clock of the module.
- `reg x = init` is a register reset to `init`. The reset is written once for all the registers of the block; the
  SystemVerilog equivalent is a reset branch that lists every register.
- `reset rstn low async;` declares a reset that is active low and asynchronous. The generated block then uses
  `negedge rstn` in its sensitivity list.

## Values with inferred widths

A `val` is a named signal whose width the compiler infers without losing bits:

- a sum has one more bit than its wider operand (`p + c` is 17 bits when `p` is 16 bits);
- a product has the sum of its operands' widths (`a * b` of two 8-bit values is 16 bits).

Use `.wrap()` when a truncation is intended. Without a `val`, every intermediate width must be written by hand.

## Values in a pipeline

A `val` inside a stage is carried to the stages that read it, like any other signal. `Mac2` writes `s` in `add` and reads
it in `out`: the compiler adds the register between them. The `.wrap()` in `out` is the only place the width is cut.

See [Pipelines](Pipelines) for the stage rules.
