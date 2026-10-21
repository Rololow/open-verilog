module Cell__8 (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    q <= d;
  end
endmodule

module Shift (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  logic [7:0] stage [5];
  assign stage[0] = d;
  Cell__8 c_0 (.clk(clk), .d(stage[0]), .q(stage[1]));
  Cell__8 c_1 (.clk(clk), .d(stage[1]), .q(stage[2]));
  Cell__8 c_2 (.clk(clk), .d(stage[2]), .q(stage[3]));
  Cell__8 c_3 (.clk(clk), .d(stage[3]), .q(stage[4]));
  assign q = stage[4];
endmodule

module Parity (
  input logic [7:0] x,
  output logic p
);
  always_comb begin
    p = 0;
    p = p ^ x[0];
    p = p ^ x[1];
    p = p ^ x[2];
    p = p ^ x[3];
    p = p ^ x[4];
    p = p ^ x[5];
    p = p ^ x[6];
    p = p ^ x[7];
  end
endmodule

module Sel (
  input logic [1:0] s,
  input logic [3:0] a,
  output logic y
);
  always_comb begin
    unique0 case (s)
      0, 1: begin
        y = a[0];
      end
      2: begin
        y = a[1] & a[2] | a[3];
      end
      default: begin
        y = (a[0] + a[1]) * 2 == 0 ? 1 : 0;
      end
    endcase
  end
endmodule
