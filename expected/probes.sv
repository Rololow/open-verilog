module Pe (
  input logic clk,
  input logic rst,
  input logic [1:0] mode,
  input logic [7:0] din,
  output logic [7:0] acc
);
  logic [7:0] prod;
  assign prod = din * 3;
  always_ff @(posedge clk) begin
    if (rst) begin
      acc <= 0;
    end else if (mode == 2) begin
      acc <= acc + prod;
    end
  end
endmodule

module Top (
  input logic clk,
  input logic rst,
  input logic [1:0] mode,
  input logic [7:0] din,
  output logic [7:0] a0,
  output logic [7:0] a1
);
  Pe p0 (.clk(clk), .rst(rst), .mode(mode), .din(din), .acc(a0));
  Pe p1 (.clk(clk), .rst(rst), .mode(2'd2), .din(din), .acc(a1));
endmodule
