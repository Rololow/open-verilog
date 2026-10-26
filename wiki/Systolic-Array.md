# Systolic array

An output-stationary systolic array multiplies two N×N matrices. Each processing element (PE) multiplies the value that
arrives from the left by the value that arrives from the top, adds it to its accumulator, and passes both values on.
The rows of A enter from the left, the columns of B from the top, and each one is delayed so that the right values meet
in each PE: row `i` is delayed `i` cycles, column `j` is delayed `j` cycles.

```ov
// TPU-style output-stationary systolic array: C = A x B for N x N matrices.
// Row i of A enters from the left delayed by i cycles, column j of B from the top delayed by j cycles
// (the classic input skew, one `.delay()` each); every PE multiplies, accumulates and passes its inputs on.
module Pe<int W = 8, int ACC = 24>(input clk, input clear, input logic [W-1:0] a_in, input logic [W-1:0] b_in,
                                   output logic [W-1:0] a_out, output logic [W-1:0] b_out, output logic [ACC-1:0] acc) {
  always_ff @(posedge clk) {
    a_out <= a_in;
    b_out <= b_in;
    acc <= clear ? 0 : (acc + a_in * b_in).wrap();
  }
}

module Systolic<int N = 4, int W = 8>(input clk, input clear, input logic [W-1:0] a [N], input logic [W-1:0] b [N],
                                      output logic [2*W+clog2(N)-1:0] c [N][N]) {
  logic [W-1:0] ah [N][N+1];                 // horizontal links (A flows right)
  logic [W-1:0] bv [N+1][N];                 // vertical links (B flows down)
  for i in 0..N {
    assign ah[i][0] = a[i].delay(i);
    assign bv[0][i] = b[i].delay(i);
  }
  for i in 0..N {
    for j in 0..N {
      Pe<W, 2*W+clog2(N)> pe(.clk, .clear, .a_in(ah[i][j]), .b_in(bv[i][j]), .a_out(ah[i][j+1]),
                             .b_out(bv[i+1][j]), .acc(c[i][j]));
    }
  }
}
```

## The parts

- `Pe`: one cell. Its two inputs are registered and passed on to the next cell; `acc` adds their product each cycle, and
  `clear` restarts it.
- `a[i].delay(i)`: the skew. Row `i` is a chain of `i` registers. With `delay(0)` the row is not delayed.
- The double loop instantiates N×N cells, and wires each cell's outputs to its right and lower neighbours
  (`ah[i][j+1]`, `bv[i+1][j]`).
- `c [N][N]`: each cell's accumulator is a result. The width of a result grows with N: `2*W+clog2(N)` bits.

The same module written in SystemVerilog needs the skew chains as explicit two-dimensional arrays and their loops. Here
the skew is one call per row and column.
