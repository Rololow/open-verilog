module Bits (
  input logic [7:0] a,
  input logic [3:0] b,
  output logic [11:0] c,
  output logic [7:0] r,
  output logic [15:0] f,
  output logic [15:0] z,
  output logic [15:0] s,
  output logic m,
  output logic l
);
  assign c = {a, b};
  assign r = {a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]};
  assign f = {2{a}};
  assign z = {{8{1'b0}}, a};
  assign s = {{8{a[7]}}, a};
  assign m = a[7];
  assign l = a[0];
endmodule

module VecSum (
  input logic [7:0] in [4],
  output logic [9:0] sum
);
  assign sum = in[0] + in[1] + (in[2] + in[3]);
endmodule
