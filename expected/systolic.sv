module Pe__8_18 (
  input logic clk,
  input logic clear,
  input logic [7:0] a_in,
  input logic [7:0] b_in,
  output logic [7:0] a_out,
  output logic [7:0] b_out,
  output logic [17:0] acc
);
  always_ff @(posedge clk) begin
    a_out <= a_in;
    b_out <= b_in;
    acc <= clear ? 0 : acc + a_in * b_in;
  end
endmodule

module Systolic (
  input logic clk,
  input logic clear,
  input logic [7:0] a [4],
  input logic [7:0] b [4],
  output logic [17:0] c [4] [4]
);
  logic [7:0] a_1_d1;
  logic [7:0] b_1_d1;
  logic [7:0] a_2_d1;
  logic [7:0] a_2_d2;
  logic [7:0] b_2_d1;
  logic [7:0] b_2_d2;
  logic [7:0] a_3_d1;
  logic [7:0] a_3_d2;
  logic [7:0] a_3_d3;
  logic [7:0] b_3_d1;
  logic [7:0] b_3_d2;
  logic [7:0] b_3_d3;
  logic [7:0] ah [4] [5];
  logic [7:0] bv [5] [4];
  always_ff @(posedge clk) begin
    a_1_d1 <= a[1];
  end
  always_ff @(posedge clk) begin
    b_1_d1 <= b[1];
  end
  always_ff @(posedge clk) begin
    a_2_d1 <= a[2];
    a_2_d2 <= a_2_d1;
  end
  always_ff @(posedge clk) begin
    b_2_d1 <= b[2];
    b_2_d2 <= b_2_d1;
  end
  always_ff @(posedge clk) begin
    a_3_d1 <= a[3];
    a_3_d2 <= a_3_d1;
    a_3_d3 <= a_3_d2;
  end
  always_ff @(posedge clk) begin
    b_3_d1 <= b[3];
    b_3_d2 <= b_3_d1;
    b_3_d3 <= b_3_d2;
  end
  assign ah[0][0] = a[0];
  assign bv[0][0] = b[0];
  assign ah[1][0] = a_1_d1;
  assign bv[0][1] = b_1_d1;
  assign ah[2][0] = a_2_d2;
  assign bv[0][2] = b_2_d2;
  assign ah[3][0] = a_3_d3;
  assign bv[0][3] = b_3_d3;
  for (genvar i = 0; i < 4; i++) begin : gen0
    for (genvar j = 0; j < 4; j++) begin : gen1
      Pe__8_18 pe (.clk(clk), .clear(clear), .a_in(ah[i][j]), .b_in(bv[i][j]), .a_out(ah[i][j + 1]), .b_out(bv[i + 1][j]), .acc(c[i][j]));
    end
  end
endmodule
