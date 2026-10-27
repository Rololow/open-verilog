# Pipelines

A `pipeline` contains only combinational stages. The compiler puts a register at every boundary and carries every
value that a later stage reads. The valid bits are generated too.

## A multiply-add in three stages

Open Verilog. The input `c` enters with the instruction, so the `add` stage reads it as `add.c`: the value that came
with this instruction. Reading it as a plain `c` is an error, because a later stage could otherwise see a value that
belongs to another instruction.

```systemverilog ov
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

A stage can hold, drop or refill the instructions in front of it. You write the condition on the stage; the compiler
writes the logic at every boundary it affects. There are three conditions:

- `stall_when(c)`: the stage and every stage before it freeze, and a bubble (an empty slot) is inserted behind the
  stage. Nothing is lost: the producer must hold its input while it is stalled.
- `flush_when(c)`: the instruction in the stage, and the instructions in the stages before it, are removed.
- `flush_younger_when(c)`: only the instructions in the stages before it are removed, not the one in the stage: an
  older instruction that must be redone.

Each condition is ignored when its stage is empty. When several conditions meet at a boundary, the first that applies
wins, in this order: reset, flush, hold (from a stall), bubble, load.

### A stall

A fetch stage waits for memory. `stall_when` freezes the stages up to `fetch` while `wait_mem` is high. A stalled
pipeline must say so to its producer, so it needs a `ready_out` port:

```systemverilog ov
pipeline Stall(input clk, input rst, input logic valid_in, output logic ready_out, input logic [7:0] a,
               input logic wait_mem, output logic [7:0] y, output logic valid_out) {
  stage fetch { val x = a + 1; stall_when(wait_mem); }
  stage exec  { val z = x * 2; }
  stage out   { assign y = z.wrap(); }
}
```

Without `ready_out`, the compiler warns: the producer cannot know when to hold its data, so an input would be lost.

### A flush

A branch is resolved in `decode`. When `taken` is high, the instructions behind it must not reach `exec`:
`flush_when` removes the instruction in `decode` and the ones before it. `taken` is an input: it is read in a later
stage, so it is written `pipe.taken` (its value now), not `taken` (see [Inputs](#inputs-read-in-a-later-stage)).

```systemverilog ov
pipeline Flush(input clk, input rst, input logic valid_in, input logic [7:0] a,
               input logic taken, output logic [7:0] y, output logic valid_out) {
  stage fetch { val x = a + 1; }
  stage decode { val d = x + 1; flush_when(pipe.taken); }
  stage exec  { val z = d + 1; }
  stage out   { assign y = z.wrap(); }
}
```

### A trap in a later stage

An exception found in `exec` must remove the instructions that were fetched after the faulting one, but not the
faulting instruction itself. `flush_younger_when` does that:

```systemverilog ov
pipeline Younger(input clk, input rst, input logic valid_in, input logic [7:0] a,
                 input logic trap, output logic [7:0] y, output logic valid_out) {
  stage fetch { val x = a + 1; }
  stage decode { val d = x + 1; }
  stage exec  { val z = d + 1; flush_younger_when(pipe.trap); }
  stage out   { assign y = z.wrap(); }
}
```

### Inputs read in a later stage

An input port enters the pipeline with the instruction, in the first stage. Read in a later stage, it must say which
value it means:

- `stage.x` (for example `decode.taken`): the value that came with this instruction, carried through the stages;
- `pipe.x`: the value of the port now, not carried, and not part of the stall checks.

Leaving it implicit is an error, because the two meanings are both common and picking the wrong one silently delays or
skews the signal.

### What this costs

The rules above are the whole of the control model, and they are checked by the compiler. They are still a model to
learn: a condition on one stage can freeze, drop or refill instructions in other stages, and the valid bits, the
carried values and the ready signals all follow from it. The compiler also generates simulation checks (an instruction
must reach the next stage, a stalled stage must not change, valid bits are never X) that run with `--asserts` and are
removed by `--no-asserts`.

For a small pipeline, the control conditions replace the logic that would otherwise be written by hand at each
boundary. For a design with many stages and controls that interact, the rules are where the time goes. A design that
only carries values from one stage to the next (no stalls, no flushes) needs only the stage declarations.

## Reset

The reset is a port or a directive. `reset rstn low async;` makes the reset asynchronous: the valid bits and the
clocked values clear on the falling edge of `rstn`, without waiting for the clock. Without `async`, they clear on the
next clock edge while the reset is high.

```systemverilog ov
pipeline Async(input clk, input rstn, input logic valid_in, input logic [7:0] a,
               output logic [7:0] y, output logic valid_out) {
  reset rstn low async;
  stage first { val x = a + 1; }
  stage out   { assign y = x.wrap(); }
}
```

## A parametric number of stages

`for` inside a pipeline generates stages, one per iteration: `step_0`, ..., `step_{N-1}`. A pipeline-level array is
split into one signal per element, and each element belongs to the stage that drives it.

```systemverilog ov
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
