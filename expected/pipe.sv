module Pipe (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  logic [7:0] stage [4];
  assign stage[0] = d;
  for (genvar i = 0; i < 3; i++) begin : gen0
    always_ff @(posedge clk) begin
      stage[i + 1] <= stage[i];
    end
  end
  assign q = stage[3];
endmodule
