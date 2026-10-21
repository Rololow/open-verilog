module Adder (
  input logic [7:0] in [4],
  output logic [9:0] sum
);
  assign sum = in[0] + in[1] + (in[2] + in[3]);
endmodule
