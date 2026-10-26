# Arrays and delays

Two common patterns: a bank of counters held for a number of cycles, and the same delay applied to every lane of an
array. The Open Verilog version states each bound once. The SystemVerilog version repeats the bounds in each loop and
writes the reset by hand.

## A bank of hold counters

Open Verilog. A `reg` array is reset to zero by the implicit clock and reset, so there is no reset code.

```ov
module BankHold<int NB = 4, int HOLD = 8>(input clk, input rst,
                                          input logic [clog2(NB)-1:0] issue_bank, input logic issue) {
  reg [7:0] hold [NB];
  always_ff {
    for bank in 0..NB {
      if (issue && issue_bank == bank) { hold[bank] <= HOLD; }
      else if (hold[bank] != 0) { hold[bank] <= hold[bank] - 1; }
    }
  }
}
```

SystemVerilog, the same behaviour:

```systemverilog
module BankHold #(parameter int NB = 4, parameter int HOLD = 8) (
  input  logic clk,
  input  logic rst,
  input  logic [$clog2(NB)-1:0] issue_bank,
  input  logic issue
);
  logic [7:0] hold [NB];
  always_ff @(posedge clk) begin
    if (rst) begin
      for (int bank = 0; bank < NB; bank++) hold[bank] <= '0;
    end else begin
      for (int bank = 0; bank < NB; bank++) begin
        if (issue && issue_bank == bank) hold[bank] <= HOLD;
        else if (hold[bank] != 0) hold[bank] <= hold[bank] - 1;
      end
    end
  end
endmodule
```

The reset loop is the part that gets forgotten: a register array without it starts at X.

## A delay on every lane

Open Verilog. `delay(D)` is a chain of `D` registers.

```ov
module Lanes<int N = 8, int W = 16, int D = 3>(input clk, input logic [W-1:0] x [N], output logic [W-1:0] y [N]) {
  for lane in 0..N { assign y[lane] = x[lane].delay(D); }
}
```

SystemVerilog, the same behaviour:

```systemverilog
module Lanes #(parameter int N = 8, parameter int W = 16, parameter int D = 3) (
  input  logic clk,
  input  logic [W-1:0] x [N],
  output logic [W-1:0] y [N]
);
  logic [W-1:0] dly [N][D];
  always_ff @(posedge clk) begin
    for (int lane = 0; lane < N; lane++) begin
      dly[lane][0] <= x[lane];
      for (int s = 1; s < D; s++) dly[lane][s] <= dly[lane][s-1];
    end
  end
  always_comb begin
    for (int lane = 0; lane < N; lane++) y[lane] = dly[lane][D-1];
  end
endmodule
```

## What changes

- The bounds (`NB`, `N`, `D`) are written once. In SystemVerilog they appear in the loop, the declaration and the reset.
- Reset is implicit for `reg`. In SystemVerilog it is a loop you have to write.
- `delay(D)` replaces the two-dimensional register array, its two loops and the index `[D-1]`.
