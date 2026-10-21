module Reg8__8 (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    q <= d;
  end
endmodule

module Bank (
  input logic clk,
  input logic [7:0] d [4],
  output logic [7:0] q [4]
);
  Reg8__8 r_0 (.clk(clk), .d(d[0]), .q(q[0]));
  Reg8__8 r_1 (.clk(clk), .d(d[1]), .q(q[1]));
  Reg8__8 r_2 (.clk(clk), .d(d[2]), .q(q[2]));
  Reg8__8 r_3 (.clk(clk), .d(d[3]), .q(q[3]));
endmodule

module Acc (
  input logic clk,
  input logic rst,
  input logic [7:0] x,
  output logic [7:0] s
);
  always @(posedge clk or negedge rst) begin
    if (!rst) begin
      s <= 0;
    end else begin
      s <= s + x;
    end
  end
endmodule
