module Counter (
  input logic clk,
  input logic rst,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    if (rst) begin
      q <= 0;
    end else begin
      q <= q + 1;
    end
  end
endmodule
