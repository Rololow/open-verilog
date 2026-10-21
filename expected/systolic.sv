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
  logic [7:0] ov_dly1_1;
  logic [7:0] ov_dly2_1;
  logic [7:0] ov_dly3_1;
  logic [7:0] ov_dly3_2;
  logic [7:0] ov_dly4_1;
  logic [7:0] ov_dly4_2;
  logic [7:0] ov_dly5_1;
  logic [7:0] ov_dly5_2;
  logic [7:0] ov_dly5_3;
  logic [7:0] ov_dly6_1;
  logic [7:0] ov_dly6_2;
  logic [7:0] ov_dly6_3;
  logic [7:0] ah [4] [5];
  logic [7:0] bv [5] [4];
  always_ff @(posedge clk) begin
    ov_dly1_1 <= a[1];
  end
  always_ff @(posedge clk) begin
    ov_dly2_1 <= b[1];
  end
  always_ff @(posedge clk) begin
    ov_dly3_1 <= a[2];
    ov_dly3_2 <= ov_dly3_1;
  end
  always_ff @(posedge clk) begin
    ov_dly4_1 <= b[2];
    ov_dly4_2 <= ov_dly4_1;
  end
  always_ff @(posedge clk) begin
    ov_dly5_1 <= a[3];
    ov_dly5_2 <= ov_dly5_1;
    ov_dly5_3 <= ov_dly5_2;
  end
  always_ff @(posedge clk) begin
    ov_dly6_1 <= b[3];
    ov_dly6_2 <= ov_dly6_1;
    ov_dly6_3 <= ov_dly6_2;
  end
  assign ah[0][0] = a[0];
  assign bv[0][0] = b[0];
  assign ah[1][0] = ov_dly1_1;
  assign bv[0][1] = ov_dly2_1;
  assign ah[2][0] = ov_dly3_2;
  assign bv[0][2] = ov_dly4_2;
  assign ah[3][0] = ov_dly5_3;
  assign bv[0][3] = ov_dly6_3;
  Pe__8_18 pe_0_0 (.clk(clk), .clear(clear), .a_in(ah[0][0]), .b_in(bv[0][0]), .a_out(ah[0][1]), .b_out(bv[1][0]), .acc(c[0][0]));
  Pe__8_18 pe_0_1 (.clk(clk), .clear(clear), .a_in(ah[0][1]), .b_in(bv[0][1]), .a_out(ah[0][2]), .b_out(bv[1][1]), .acc(c[0][1]));
  Pe__8_18 pe_0_2 (.clk(clk), .clear(clear), .a_in(ah[0][2]), .b_in(bv[0][2]), .a_out(ah[0][3]), .b_out(bv[1][2]), .acc(c[0][2]));
  Pe__8_18 pe_0_3 (.clk(clk), .clear(clear), .a_in(ah[0][3]), .b_in(bv[0][3]), .a_out(ah[0][4]), .b_out(bv[1][3]), .acc(c[0][3]));
  Pe__8_18 pe_1_0 (.clk(clk), .clear(clear), .a_in(ah[1][0]), .b_in(bv[1][0]), .a_out(ah[1][1]), .b_out(bv[2][0]), .acc(c[1][0]));
  Pe__8_18 pe_1_1 (.clk(clk), .clear(clear), .a_in(ah[1][1]), .b_in(bv[1][1]), .a_out(ah[1][2]), .b_out(bv[2][1]), .acc(c[1][1]));
  Pe__8_18 pe_1_2 (.clk(clk), .clear(clear), .a_in(ah[1][2]), .b_in(bv[1][2]), .a_out(ah[1][3]), .b_out(bv[2][2]), .acc(c[1][2]));
  Pe__8_18 pe_1_3 (.clk(clk), .clear(clear), .a_in(ah[1][3]), .b_in(bv[1][3]), .a_out(ah[1][4]), .b_out(bv[2][3]), .acc(c[1][3]));
  Pe__8_18 pe_2_0 (.clk(clk), .clear(clear), .a_in(ah[2][0]), .b_in(bv[2][0]), .a_out(ah[2][1]), .b_out(bv[3][0]), .acc(c[2][0]));
  Pe__8_18 pe_2_1 (.clk(clk), .clear(clear), .a_in(ah[2][1]), .b_in(bv[2][1]), .a_out(ah[2][2]), .b_out(bv[3][1]), .acc(c[2][1]));
  Pe__8_18 pe_2_2 (.clk(clk), .clear(clear), .a_in(ah[2][2]), .b_in(bv[2][2]), .a_out(ah[2][3]), .b_out(bv[3][2]), .acc(c[2][2]));
  Pe__8_18 pe_2_3 (.clk(clk), .clear(clear), .a_in(ah[2][3]), .b_in(bv[2][3]), .a_out(ah[2][4]), .b_out(bv[3][3]), .acc(c[2][3]));
  Pe__8_18 pe_3_0 (.clk(clk), .clear(clear), .a_in(ah[3][0]), .b_in(bv[3][0]), .a_out(ah[3][1]), .b_out(bv[4][0]), .acc(c[3][0]));
  Pe__8_18 pe_3_1 (.clk(clk), .clear(clear), .a_in(ah[3][1]), .b_in(bv[3][1]), .a_out(ah[3][2]), .b_out(bv[4][1]), .acc(c[3][1]));
  Pe__8_18 pe_3_2 (.clk(clk), .clear(clear), .a_in(ah[3][2]), .b_in(bv[3][2]), .a_out(ah[3][3]), .b_out(bv[4][2]), .acc(c[3][2]));
  Pe__8_18 pe_3_3 (.clk(clk), .clear(clear), .a_in(ah[3][3]), .b_in(bv[3][3]), .a_out(ah[3][4]), .b_out(bv[4][3]), .acc(c[3][3]));
endmodule
