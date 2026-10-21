package pixel_pkg;
  typedef struct packed {
    logic [7:0] r;
    logic [7:0] g;
    logic [7:0] b;
  } pixel_t;
endpackage

module RegFile (
  input logic clk,
  input logic rst,
  input logic we,
  input logic [1:0] wa,
  input logic [7:0] wd,
  input logic [1:0] ov_read_0_i,
  output logic [7:0] ov_read_0,
  input logic [1:0] ov_read_1_i,
  output logic [7:0] ov_read_1,
  input logic [1:0] ov_read_2_i,
  output logic [7:0] ov_read_2
);
  logic [7:0] mem [4];
  always_ff @(posedge clk) begin
    if (we) begin
      mem[wa] <= wd;
    end
  end
  assign ov_read_0 = mem[ov_read_0_i];
  assign ov_read_1 = mem[ov_read_1_i];
  assign ov_read_2 = mem[ov_read_2_i];
endmodule

module Dual (
  input logic clk,
  input logic rst,
  input logic we,
  input logic [1:0] wa,
  input logic [7:0] wd,
  input logic [1:0] ra,
  input logic [1:0] rb,
  output logic [7:0] a,
  output logic [7:0] b,
  output logic [7:0] last
);
  logic [7:0] rf__read_0;
  logic [7:0] rf__read_1;
  logic [7:0] rf__read_2;
  RegFile rf (.clk(clk), .rst(rst), .we(we), .wa(wa), .wd(wd), .ov_read_0_i(ra), .ov_read_0(rf__read_0), .ov_read_1_i(rb), .ov_read_1(rf__read_1), .ov_read_2_i(2'd3), .ov_read_2(rf__read_2));
  assign a = rf__read_0;
  assign b = rf__read_1;
  assign last = rf__read_2;
endmodule

module Luma (
  input pixel_pkg::pixel_t p,
  input logic [7:0] k,
  output logic [9:0] s,
  output logic bright
);
  assign s = 10'(p.r + p.g + p.b);
  assign bright = 10'(p.r + p.g + p.b) > k;
endmodule
