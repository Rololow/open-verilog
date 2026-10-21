module Sync__5 (
  input logic clk,
  input logic [4:0] d,
  output logic [4:0] q
);
  logic [4:0] meta;
  always_ff @(posedge clk) begin
    meta <= d;
    q <= meta;
  end
endmodule

module AsyncFifo__8_4 (
  input logic wclk,
  input logic wrst,
  input logic push,
  input logic [7:0] wdata,
  output logic full,
  input logic rclk,
  input logic rrst,
  input logic pop,
  output logic [7:0] rdata,
  output logic empty
);
  logic [7:0] mem [16];
  logic [4:0] wbin;
  logic [4:0] rbin;
  logic [4:0] wgray;
  logic [4:0] rgray;
  logic [4:0] wgray_r;
  logic [4:0] rgray_w;
  wire [4:0] wbin_next;
  wire [4:0] rbin_next;
  Sync__5 sync_w2r (.clk(rclk), .d(wgray), .q(wgray_r));
  Sync__5 sync_r2w (.clk(wclk), .d(rgray), .q(rgray_w));
  assign wbin_next = wbin + (push && !full);
  assign rbin_next = rbin + (pop && !empty);
  always_ff @(posedge wclk) begin
    if (wrst) begin
      {wbin, wgray, full} <= 0;
    end else begin
      if (push && !full) begin
        mem[wbin[3:0]] <= wdata;
      end
      wbin <= wbin_next;
      wgray <= wbin_next ^ wbin_next >> 1;
      full <= (wbin_next ^ wbin_next >> 1) == {~rgray_w[4:3], rgray_w[2:0]};
    end
  end
  always_ff @(posedge rclk) begin
    if (rrst) begin
      {rbin, rgray} <= 0;
      empty <= 1;
    end else begin
      rbin <= rbin_next;
      rgray <= rbin_next ^ rbin_next >> 1;
      empty <= (rbin_next ^ rbin_next >> 1) == wgray_r;
    end
  end
  assign rdata = mem[rbin[3:0]];
endmodule

module Sync__1 (
  input logic clk,
  input logic [0:0] d,
  output logic [0:0] q
);
  logic [0:0] meta;
  always_ff @(posedge clk) begin
    meta <= d;
    q <= meta;
  end
endmodule

module Bridge (
  input logic clk_a,
  input logic rst_a,
  input logic clk_b,
  input logic rst_b,
  input logic [7:0] sample,
  input logic start,
  output logic [15:0] sum,
  output logic started
);
  logic push;
  logic full;
  logic empty;
  logic [7:0] data;
  logic go_b;
  always_ff @(posedge clk_a) begin
    push <= !rst_a && !full;
  end
  AsyncFifo__8_4 fifo (.wclk(clk_a), .wrst(rst_a), .push(push), .wdata(sample), .full(full), .rclk(clk_b), .rrst(rst_b), .pop(!empty), .rdata(data), .empty(empty));
  Sync__1 go (.clk(clk_b), .d(start), .q(go_b));
  always_ff @(posedge clk_b) begin
    if (rst_b) begin
      sum <= 0;
      started <= 0;
    end else begin
      started <= go_b;
      if (!empty) begin
        sum <= sum + data;
      end
    end
  end
endmodule
