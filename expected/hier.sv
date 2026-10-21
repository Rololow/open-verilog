module Counter (
  input logic clk,
  input logic rst,
  input logic hold = 0,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    if (rst) begin
      q <= 0;
    end else if (!hold) begin
      q <= q + 1;
    end
  end
endmodule

module Cluster (
  input logic clk,
  input logic rst,
  output logic [7:0] sum,
  output logic [7:0] ov_tap_c_2_q,
  input logic ov_drv_c_2_hold = 0
);
  logic [7:0] q [4];
  Counter c_0 (.clk(clk), .rst(rst), .q(q[0]));
  Counter c_1 (.clk(clk), .rst(rst), .q(q[1]));
  Counter c_2 (.clk(clk), .rst(rst), .q(q[2]), .hold(ov_drv_c_2_hold));
  Counter c_3 (.clk(clk), .rst(rst), .q(q[3]));
  assign sum = q[0] ^ q[1] ^ q[2] ^ q[3];
  assign ov_tap_c_2_q = q[2];
endmodule

module Soc (
  input logic clk,
  input logic rst,
  input logic dbg_halt,
  output logic [7:0] sum,
  output logic [7:0] probe
);
  logic [7:0] ov_tap_cluster_c_2_q;
  Cluster cluster (.clk(clk), .rst(rst), .sum(sum), .ov_tap_c_2_q(ov_tap_cluster_c_2_q), .ov_drv_c_2_hold(dbg_halt));
  assign probe = ov_tap_cluster_c_2_q;
endmodule
