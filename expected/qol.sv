module Counter2 (
  input logic clk,
  input logic rst,
  input logic en,
  output logic [7:0] q
);
  logic [7:0] cnt;
  always_ff @(posedge clk) begin
    if (rst) begin
      cnt <= 0;
    end else begin
      if (en) begin
        cnt <= cnt + 1;
      end
    end
  end
  assign q = cnt;
endmodule

module Flag (
  input logic clk,
  input logic rstn,
  input logic set,
  input logic clr,
  output logic f
);
  logic r;
  always_ff @(posedge clk or negedge rstn) begin
    if (!rstn) begin
      r <= 0;
    end else begin
      if (set) begin
        r <= 1;
      end else if (clr) begin
        r <= 0;
      end
    end
  end
  assign f = r;
endmodule

module Mac3 (
  input logic [7:0] a,
  input logic [7:0] b,
  input logic [7:0] c,
  output logic [16:0] y
);
  logic [15:0] p;
  logic [16:0] s;
  assign p = a * b;
  assign s = p + c;
  assign y = s;
endmodule

module Mac2 (
  input logic clk,
  input logic rst,
  input logic valid_in,
  input logic [7:0] a,
  input logic [7:0] b,
  input logic [7:0] c,
  output logic [15:0] y,
  output logic valid_out
);
  logic ov_valid_0;
  logic ov_valid_1;
  logic ov_valid_2;
  logic [15:0] p__s1;
  logic [7:0] c__s1;
  logic [16:0] s__s2;
  logic [15:0] p;
  logic [16:0] s;
  assign p = a * b;
  assign s = p__s1 + c__s1;
  assign y = s__s2;
  assign ov_valid_0 = valid_in;
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_1 <= 1'b0;
    end else begin
      ov_valid_1 <= ov_valid_0;
    end
    p__s1 <= p;
    c__s1 <= c;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_2 <= 1'b0;
    end else begin
      ov_valid_2 <= ov_valid_1;
    end
    s__s2 <= s;
  end
  assign valid_out = ov_valid_2;
  `ifndef SYNTHESIS
  // ovc: pipeline checks for 'Mac2' (nothing may be lost)
  ov_a_advance_1: assert property (@(posedge clk) disable iff (rst) ov_valid_0 |=> ov_valid_1)
    else $error("Mac2: an instruction leaving stage 'mul' did not reach stage 'add'");
  ov_a_advance_2: assert property (@(posedge clk) disable iff (rst) ov_valid_1 |=> ov_valid_2)
    else $error("Mac2: an instruction leaving stage 'add' did not reach stage 'out'");
  ov_a_valid_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({ov_valid_1, ov_valid_2}))
    else $error("Mac2: a valid bit is X (missing reset?)");
  `endif
endmodule
