# Debug, hierarchy and tests

## Reaching into the hierarchy

A module can read a signal deep inside its instances, and drive an input that has a default, without editing the modules
in between. The compiler adds the ports on the way up and down.

```systemverilog ov
module Counter(input clk, input rst, input hold = 0, output logic [7:0] q) {
  always_ff @(posedge clk) { if (rst) { q <= 0; } else if (!hold) { q <= q + 1; } }
}
module Cluster(input clk, input rst, output logic [7:0] sum) {
  logic [7:0] q [4];
  for i in 0..4 { Counter c(.clk, .rst, .q(q[i])); }
  assign sum = q.list().reduce(^);
}
module Soc(input clk, input rst, input dbg_halt, output logic [7:0] sum, output logic [7:0] probe) {
  Cluster cluster(.clk, .rst, .sum);
  assign probe = cluster.c[2].q;          // read three levels down: ports are added on the way up
  assign cluster.c[2].hold = dbg_halt;    // drive an input that has a default: ports are added on the way down
}
```

- `cluster.c[2].q` is read in `Soc`, two levels above the counter. The compiler adds the ports that carry it through
  `Cluster` up to `Soc`.
- `cluster.c[2].hold` drives an input that has a default value. The port is added on the way down, so `dbg_halt` stops
  one counter without touching `Cluster`.

## Probes

A probe is debug logic kept out of the design. It is written in its own block, bound into every instance of a module
when the probe file is emitted, and removed from the design itself.

```systemverilog ov
module Pe(input clk, input rst, input logic [1:0] mode, input logic [7:0] din, output logic [7:0] acc) {
  logic [7:0] prod;
  assign prod = (din * 3).wrap();
  always_ff @(posedge clk) { if (rst) { acc <= 0; } else if (mode == 2) { acc <= (acc + prod).wrap(); } }
}
module Top(input clk, input rst, input logic [1:0] mode, input logic [7:0] din, output logic [7:0] a0, output logic [7:0] a1) {
  Pe p0(.clk, .rst, .mode, .din, .acc(a0));
  Pe p1(.clk, .rst, .mode(2'd2), .din, .acc(a1));
}

probe pe_acc on Pe when (mode == 2) limit 3 {
  print("din=%0d prod=%0d acc=%0d", din, prod, acc);
}

test "accumulates" for Top {
  din = 5; mode = 2;
  step(4);
  expect(a0 == 60);
}
```

- `probe pe_acc on Pe`: the probe reads the signals of `Pe` by name (`din`, `prod`, `acc`). `when` restricts it to the
  cycles where the condition holds; `limit 3` prints at most three times.
- `ovc design.ov --emit-probes probes.sv` writes the probes as a module bound into every `Pe`. Simulate with the extra
  top module `ovc_probes`, and the plusarg `+ovc_probe` to show them all, or `+ovc_probe=pe_acc` for one.

## Tests

A `test` block drives a module and checks its outputs. It is written next to the design.

```systemverilog ov
module Counter<int W = 8>(input clk, input rst, input en, output logic [W-1:0] q) {
  always_ff @(posedge clk) { if (rst) { q <= 0; } else if (en) { q <= (q + 1).wrap(); } }
}

module Add(input logic [7:0] a, input logic [7:0] b, output logic [8:0] s) { assign s = a + b; }

test "counts and wraps" for Counter<8> {
  en = 1;
  step(3);
  expect(q == 3);
  step(253);
  expect(q == 0, "wraps after 256");
}

test "holds when disabled" for Counter<4> timeout 50 {
  en = 1;
  step(2);
  en = 0;
  step(5);
  expect(q == 2);
  en = 1;
  wait_until(q == 9, 10);
}

test "adds with carry" for Add {
  for i in 0..3 {
    a = 80 * i;
    b = 7;
    expect(s == 80 * i + 7);
  }
  a = 255;
  b = 255;
  expect(s == 510);
}
```

- `for Counter<8>`: the module under test. Its ports are the names used in the block.
- `step(n)` advances `n` cycles. `expect(cond, "message")` checks a condition. `wait_until(cond, n)` waits up to `n`
  cycles for a condition.
- `timeout 50` is a cycle limit for the test.
- A `for` loop in a test runs its checks once per value.

`ovc testbench.ov --emit-tests tests.sv` writes a testbench with top module `ovc_tests`. Any SystemVerilog simulator runs
it, and it prints one `PASS` or `FAIL` line per test, then a summary.
