module Counter (
  input logic clk,
  input logic rst,
  input logic en,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    if (rst) begin
      q <= 0;
    end else if (en) begin
      q <= q + 1;
    end
  end
endmodule

module Add (
  input logic [7:0] a,
  input logic [7:0] b,
  output logic [8:0] s
);
  assign s = a + b;
endmodule

module Counter__4 (
  input logic clk,
  input logic rst,
  input logic en,
  output logic [3:0] q
);
  always_ff @(posedge clk) begin
    if (rst) begin
      q <= 0;
    end else if (en) begin
      q <= q + 1;
    end
  end
endmodule
