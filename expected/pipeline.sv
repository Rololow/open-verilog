module Mac (
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
  logic [15:0] s__s2;
  logic [15:0] p;
  logic [15:0] s;
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
  // ovc: pipeline checks for 'Mac' (nothing may be lost)
  ov_a_advance_1: assert property (@(posedge clk) disable iff (rst) ov_valid_0 |=> ov_valid_1)
    else $error("Mac: an instruction leaving stage 'mul' did not reach stage 'add'");
  ov_a_advance_2: assert property (@(posedge clk) disable iff (rst) ov_valid_1 |=> ov_valid_2)
    else $error("Mac: an instruction leaving stage 'add' did not reach stage 'out'");
  ov_a_valid_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({ov_valid_1, ov_valid_2}))
    else $error("Mac: a valid bit is X (missing reset?)");
  `endif
endmodule

module Slow (
  input logic clk,
  input logic rst,
  input logic valid_in,
  input logic [7:0] d,
  input logic stall,
  output logic [7:0] y,
  output logic valid_out,
  output logic ready_out
);
  logic ov_valid_0;
  logic ov_valid_1;
  logic ov_valid_2;
  logic ov_stall_1;
  logic ov_hold_1;
  logic ov_hold_0;
  logic [7:0] a__s1;
  logic [7:0] b__s2;
  logic [7:0] a;
  logic [7:0] b;
  assign a = d + 1;
  assign b = a__s1 + 1;
  assign y = b__s2 + 1;
  assign ov_valid_0 = valid_in;
  assign ov_stall_1 = ov_valid_1 && stall;
  assign ov_hold_1 = ov_stall_1;
  assign ov_hold_0 = ov_hold_1;
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_1 <= 1'b0;
    end else if (!ov_hold_1) begin
      ov_valid_1 <= ov_valid_0;
    end
    if (!ov_hold_1) begin
      a__s1 <= a;
    end
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_2 <= 1'b0;
    end else begin
      ov_valid_2 <= ov_valid_1 & ~ov_stall_1;
    end
    b__s2 <= b;
  end
  assign valid_out = ov_valid_2;
  assign ready_out = ~ov_hold_0;
  `ifndef SYNTHESIS
  // ovc: pipeline checks for 'Slow' (nothing may be lost)
  ov_a_advance_1: assert property (@(posedge clk) disable iff (rst) ov_valid_0 && !ov_hold_1 |=> ov_valid_1)
    else $error("Slow: an instruction leaving stage 's0' did not reach stage 's1'");
  ov_a_hold_1: assert property (@(posedge clk) disable iff (rst) ov_hold_1 |=> $stable(ov_valid_1) && $stable(a__s1))
    else $error("Slow: stage 's1' changed while it was stalled");
  ov_a_advance_2: assert property (@(posedge clk) disable iff (rst) ov_valid_1 && !ov_stall_1 |=> ov_valid_2)
    else $error("Slow: an instruction leaving stage 's1' did not reach stage 's2'");
  ov_a_input_held: assert property (@(posedge clk) disable iff (rst) valid_in && ov_hold_0 |=> valid_in && $stable(d))
    else $error("Slow: the producer changed valid_in or its data while the pipeline was stalled (ready_out = 0): that input is lost");
  ov_a_valid_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({ov_valid_1, ov_valid_2}))
    else $error("Slow: a valid bit is X (missing reset?)");
  `endif
endmodule

module Flushy (
  input logic clk,
  input logic rst,
  input logic valid_in,
  input logic [7:0] d,
  input logic kill,
  output logic [7:0] y,
  output logic valid_out
);
  logic ov_valid_0;
  logic ov_valid_1;
  logic ov_valid_2;
  logic ov_flush_1;
  logic ov_kill_2;
  logic ov_kill_1;
  logic [7:0] a__s1;
  logic [7:0] b__s2;
  logic [7:0] a;
  logic [7:0] b;
  assign a = d;
  assign b = a__s1;
  assign y = b__s2;
  assign ov_valid_0 = valid_in;
  assign ov_flush_1 = ov_valid_1 && kill;
  assign ov_kill_2 = ov_flush_1;
  assign ov_kill_1 = ov_kill_2;
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_1 <= 1'b0;
    end else if (ov_kill_1) begin
      ov_valid_1 <= 1'b0;
    end else begin
      ov_valid_1 <= ov_valid_0;
    end
    a__s1 <= a;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_2 <= 1'b0;
    end else if (ov_kill_2) begin
      ov_valid_2 <= 1'b0;
    end else begin
      ov_valid_2 <= ov_valid_1;
    end
    b__s2 <= b;
  end
  assign valid_out = ov_valid_2;
  `ifndef SYNTHESIS
  // ovc: pipeline checks for 'Flushy' (nothing may be lost)
  ov_a_advance_1: assert property (@(posedge clk) disable iff (rst) ov_valid_0 && !ov_kill_1 |=> ov_valid_1)
    else $error("Flushy: an instruction leaving stage 's0' did not reach stage 's1'");
  ov_a_advance_2: assert property (@(posedge clk) disable iff (rst) ov_valid_1 && !ov_kill_2 |=> ov_valid_2)
    else $error("Flushy: an instruction leaving stage 's1' did not reach stage 's2'");
  ov_a_valid_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({ov_valid_1, ov_valid_2}))
    else $error("Flushy: a valid bit is X (missing reset?)");
  `endif
endmodule
