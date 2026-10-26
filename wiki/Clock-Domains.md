# Clock domains

A design can have several clocks. The compiler works out the clock domain of every signal from the clocks of the
registers that drive and read it. A value can only change domain in two ways:

- through an `async` input, which is a synchronizer: the one place where a signal from another domain may enter;
- through an `async` declaration, for storage whose stability the design guarantees, such as the memory of a FIFO.

Anything else that crosses a domain is an error.

```systemverilog ov
// two-flop synchronizer: the only place where `d` may come from another domain
module Sync<int W = 1>(input clk, input async logic [W-1:0] d, output logic [W-1:0] q) {
  logic [W-1:0] meta;
  always_ff @(posedge clk) { meta <= d; q <= meta; }
}

fn gray(b) { return b ^ (b >> 1); }

// asynchronous FIFO: gray-coded pointers cross through synchronizers, the memory is read in the other domain
module AsyncFifo<int W = 8, int A = 4>(
  input wclk, input wrst, input push, input logic [W-1:0] wdata, output logic full,
  input rclk, input rrst, input pop, output logic [W-1:0] rdata, output logic empty
) {
  async logic [W-1:0] mem [1 << A];      // written in wclk, read in rclk: safe while the pointers say so
  logic [A:0] wbin, rbin, wgray, rgray, wgray_r, rgray_w;
  Sync<A + 1> sync_w2r(.clk(rclk), .d(wgray), .q(wgray_r));
  Sync<A + 1> sync_r2w(.clk(wclk), .d(rgray), .q(rgray_w));
  wire [A:0] wbin_next = (wbin + (push && !full)).wrap();
  wire [A:0] rbin_next = (rbin + (pop && !empty)).wrap();
  always_ff @(posedge wclk) {
    if (wrst) { {wbin, wgray, full} <= 0; }
    else {
      if (push && !full) { mem[wbin[A-1:0]] <= wdata; }
      wbin <= wbin_next;
      wgray <= gray(wbin_next);
      full <= gray(wbin_next) == {~rgray_w[A:A-1], rgray_w[A-2:0]};
    }
  }
  always_ff @(posedge rclk) {
    if (rrst) { {rbin, rgray} <= 0; empty <= 1; }
    else {
      rbin <= rbin_next;
      rgray <= gray(rbin_next);
      empty <= gray(rbin_next) == wgray_r;
    }
  }
  assign rdata = mem[rbin[A-1:0]];
}

// a producer on clk_a, a consumer on clk_b
module Bridge(input clk_a, input rst_a, input clk_b, input rst_b, input logic [7:0] sample, input start,
              output logic [15:0] sum, output logic started) {
  logic push, full, empty;
  logic [7:0] data;
  logic go_b;
  always_ff @(posedge clk_a) { push <= !rst_a && !full; }
  AsyncFifo<8, 4> fifo(.wclk(clk_a), .wrst(rst_a), .push, .wdata(sample), .full,
                       .rclk(clk_b), .rrst(rst_b), .pop(!empty), .rdata(data), .empty);
  Sync go(.clk(clk_b), .d(start), .q(go_b));                 // a level from outside, into clk_b
  always_ff @(posedge clk_b) {
    if (rst_b) { sum <= 0; started <= 0; }
    else {
      started <= go_b;
      if (!empty) { sum <= sum + data; }
    }
  }
}
```

## The parts

- `Sync`: a two-flop synchronizer. Its input is `async`, so it is the only module input that may come from another
  domain without an error.
- `AsyncFifo`: the write pointer and the read pointer are Gray-coded. Each crosses to the other domain through a
  `Sync`. `mem` is `async` because it is written in `wclk` and read in `rclk`, which is safe only while the pointers
  say so.
- `Bridge`: a producer on `clk_a` and a consumer on `clk_b`. `start` comes from outside and goes through a `Sync`
  before it is used in `clk_b`.

The examples do not show what an error looks like. A register of `clk_b` that reads a register of `clk_a` directly is a
crossing: the compiler refuses it with the message `clock-domain crossing in module ...`.
