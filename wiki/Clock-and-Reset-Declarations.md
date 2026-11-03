# Clock and reset declarations

A module declares which of its signals are its clock and its reset. The compiler uses only these declarations: no
name is recognised (`clk`, `rst`, `rst_n`... are ordinary port names).

```systemverilog ov
module Counter(input clk, input rst, output logic [7:0] q) {
  clock clk;
  reset rst;

  reg [7:0] cnt = 0;
  always_ff { cnt <= cnt + 1; }
  assign q = cnt;
}
```

- `clock NAME;` is the clock of the module's clocked blocks: `always_ff`, registers (`reg`), `fsm`, pipelines, the delay
  lines of `.delay()`, the protocol monitors and the probes.
- `reset NAME [low] [async];` is the reset. `low` makes it active low (`rst_n`), `async` makes it asynchronous.

```systemverilog ov
module Flag(input clk, input rst_n, input set, output logic f) {
  clock clk;
  reset rst_n low async;

  reg r = 0;
  always_ff { if (set) { r <= 1; } }
  assign f = r;
}
```

The declarations go at the top of the module body. A module without clocked blocks needs none. A module without a
`reset` declaration has no reset branch: its registers are only written by their clocked blocks.

## Errors

A clocked block whose module declares no clock is an error, reported at the block:

| Message | Where | Declare |
| --- | --- | --- |
| `a clocked block needs the module's clock: declare \`clock clk;\` in the module` | an `always_ff`, a `reg` | `clock` |
| `an fsm needs the module's clock: ...` | an `fsm` | `clock` |
| `a delay register needs the module's clock: ...` | a `.delay(n)` | `clock` |
| `a protocol monitor needs the module's clock: ...` | a bundle port with a monitor | `clock` |
| `probe 'p': module 'M' has no clock to sample on: ...` | a `probe` | `clock` in the target |
| `pipeline 'P' has no clock: declare \`clock clk;\`` | a pipeline | `clock` |
| `pipeline 'P' has no reset: declare \`reset rst;\`` | a pipeline with more than one stage | `reset` |
| `pipeline 'P' is clocked by 'x', which is not a port` | `clock x;` names no port | a port `x` |

## The reset checks

The reset checks (see [Clock domains](Clock-Domains) and [Reset and quality of life](Reset-and-Quality-of-Life)) read the
declared reset only: a reset used both asynchronously and synchronously, or with both polarities, is reported against
the declared name.
