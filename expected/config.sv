module Datapath__CpuCfg (
  input logic [31:0] a [2],
  input logic [31:0] b [2],
  output logic [31:0] y [2],
  output logic [32:0] total
);
  assign y[0] = a[0] + b[0];
  assign y[1] = a[1] + b[1];
  assign total = y[0] + y[1];
endmodule

module Datapath__CpuCfg_lanes4 (
  input logic [31:0] a [4],
  input logic [31:0] b [4],
  output logic [31:0] y [4],
  output logic [33:0] total
);
  assign y[0] = a[0] + b[0];
  assign y[1] = a[1] + b[1];
  assign y[2] = a[2] + b[2];
  assign y[3] = a[3] + b[3];
  assign total = y[0] + y[1] + (y[2] + y[3]);
endmodule

module Datapath__BigCfg (
  input logic [63:0] a [2],
  input logic [63:0] b [2],
  output logic [63:0] y [2],
  output logic [64:0] total
);
  assign y[0] = a[0] * b[0];
  assign y[1] = a[1] * b[1];
  assign total = y[0] + y[1];
endmodule

module Top (
  input logic [31:0] a2 [2],
  input logic [31:0] b2 [2],
  output logic [31:0] y2 [2],
  output logic [32:0] t2,
  input logic [31:0] a4 [4],
  input logic [31:0] b4 [4],
  output logic [31:0] y4 [4],
  output logic [33:0] t4,
  input logic [63:0] a8 [2],
  input logic [63:0] b8 [2],
  output logic [63:0] y8 [2],
  output logic [64:0] t8
);
  Datapath__CpuCfg d2 (.a(a2), .b(b2), .y(y2), .total(t2));
  Datapath__CpuCfg_lanes4 d4 (.a(a4), .b(b4), .y(y4), .total(t4));
  Datapath__BigCfg d8 (.a(a8), .b(b8), .y(y8), .total(t8));
endmodule
