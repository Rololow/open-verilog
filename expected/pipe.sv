module Pipe (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  logic [7:0] stage [4];
  assign stage[0] = d;
  always_ff @(posedge clk) begin
    stage[1] <= stage[0];
  end
  always_ff @(posedge clk) begin
    stage[2] <= stage[1];
  end
  always_ff @(posedge clk) begin
    stage[3] <= stage[2];
  end
  assign q = stage[3];
endmodule
