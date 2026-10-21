module Widths (
  input logic [7:0] a,
  input logic [7:0] b,
  input logic [7:0] arr [3],
  output logic [10:0] sum
);
  assign sum = a + b + arr[0] + arr[1] + arr[2];
endmodule
