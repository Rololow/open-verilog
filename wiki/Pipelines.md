# Pipelines

A `pipeline` contains only combinational stages. The compiler puts a register at every boundary and carries every
value that a later stage reads. The valid bits are generated too.

## A multiply-add in three stages

Open Verilog. The input `c` enters with the instruction, so the `add` stage reads it as `add.c`: the value that came
with this instruction. Reading it as a plain `c` is an error, because a later stage could otherwise see a value that
belongs to another instruction.

```ov
pipeline Mac(input clk, input rst, input logic valid_in,
             input logic [7:0] a, input logic [7:0] b, input logic [7:0] c,
             output logic [15:0] y, output logic valid_out) {
  stage mul { val p = a * b; }
  stage add { val s = p + add.c; }
  stage out { assign y = s; }
}
```

SystemVerilog, written by hand. The behaviour is the same as the compiler's output; it is checked by simulation over
random inputs.

```systemverilog
module Mac (
  input  logic        clk,
  input  logic        rst,
  input  logic        valid_in,
  input  logic [7:0]  a,
  input  logic [7:0]  b,
  input  logic [7:0]  c,
  output logic [15:0] y,
  output logic        valid_out
);
  logic        v_add, v_out;
  logic [7:0]  c_add;
  logic [15:0] p_add;
  logic [16:0] s_out;
  wire  [15:0] p = a * b;
  wire  [16:0] s = p_add + c_add;

  always_ff @(posedge clk) begin
    if (rst) begin
      v_add <= 1'b0;
      v_out <= 1'b0;
    end else begin
      v_add <= valid_in;
      v_out <= v_add;
    end
    c_add <= c;
    p_add <= p;
    s_out <= s;
  end

  assign y         = s_out[15:0];
  assign valid_out = v_out;
endmodule
```

The compiler also emits assertions inside `` `ifndef SYNTHESIS `` that check that no instruction is lost between two
stages. The hand-written version above has none of them.

## What the compiler decides

- where each register goes, and which values it carries (`c` is delayed to the `add` stage, `s` to `out`);
- one valid bit per stage, reset with `rst`;
- `valid_in` and `valid_out` are wired to the first and last stage.

## Control

Stalls and flushes are written as conditions on a stage, not as logic at each boundary:

- `stall_when(c)` freezes the stage and the stages before it, and inserts a bubble behind it;
- `flush_when(c)` removes the instruction from the stage and from the stages before it;
- `flush_younger_when(c)` removes the instructions in the stages before it only.

The priority at each boundary is: reset, flush, hold, bubble, load.

## A parametric number of stages

`for` inside a pipeline generates stages, one per iteration: `step_0`, ..., `step_{N-1}`. A pipeline-level array is
split into one signal per element, and each element belongs to the stage that drives it.

```ov
pipeline AddChain<int N = 4>(
  input clk, input rst, input logic valid_in, input logic [7:0] d,
  output logic [7:0] y, output logic valid_out
) {
  logic [7:0] v [N+1];
  stage first { assign v[0] = d; }
  for i in 0..N {
    stage step { assign v[i+1] = (v[i] + 1).wrap(); }       // stage step_i, reads the previous stage's value
  }
  stage last { assign y = v[N]; }
}
```

Changing `N` changes the number of stages and registers, and nothing else in the code.
