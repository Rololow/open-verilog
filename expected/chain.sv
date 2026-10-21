module AddChain (
  input logic clk,
  input logic rst,
  input logic valid_in,
  input logic [7:0] d,
  output logic [7:0] y,
  output logic valid_out
);
  logic ov_valid_0;
  logic ov_valid_1;
  logic ov_valid_2;
  logic ov_valid_3;
  logic ov_valid_4;
  logic ov_valid_5;
  logic [7:0] v_0__s1;
  logic [7:0] v_1__s2;
  logic [7:0] v_2__s3;
  logic [7:0] v_3__s4;
  logic [7:0] v_4__s5;
  logic [7:0] v_0;
  logic [7:0] v_1;
  logic [7:0] v_2;
  logic [7:0] v_3;
  logic [7:0] v_4;
  assign v_0 = d;
  assign v_1 = v_0__s1 + 1;
  assign v_2 = v_1__s2 + 1;
  assign v_3 = v_2__s3 + 1;
  assign v_4 = v_3__s4 + 1;
  assign y = v_4__s5;
  assign ov_valid_0 = valid_in;
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_1 <= 1'b0;
    end else begin
      ov_valid_1 <= ov_valid_0;
    end
    v_0__s1 <= v_0;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_2 <= 1'b0;
    end else begin
      ov_valid_2 <= ov_valid_1;
    end
    v_1__s2 <= v_1;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_3 <= 1'b0;
    end else begin
      ov_valid_3 <= ov_valid_2;
    end
    v_2__s3 <= v_2;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_4 <= 1'b0;
    end else begin
      ov_valid_4 <= ov_valid_3;
    end
    v_3__s4 <= v_3;
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      ov_valid_5 <= 1'b0;
    end else begin
      ov_valid_5 <= ov_valid_4;
    end
    v_4__s5 <= v_4;
  end
  assign valid_out = ov_valid_5;
  `ifndef SYNTHESIS
  // ovc: pipeline checks for 'AddChain' (nothing may be lost)
  ov_a_advance_1: assert property (@(posedge clk) disable iff (rst) ov_valid_0 |=> ov_valid_1)
    else $error("AddChain: an instruction leaving stage 'first' did not reach stage 'step_0'");
  ov_a_advance_2: assert property (@(posedge clk) disable iff (rst) ov_valid_1 |=> ov_valid_2)
    else $error("AddChain: an instruction leaving stage 'step_0' did not reach stage 'step_1'");
  ov_a_advance_3: assert property (@(posedge clk) disable iff (rst) ov_valid_2 |=> ov_valid_3)
    else $error("AddChain: an instruction leaving stage 'step_1' did not reach stage 'step_2'");
  ov_a_advance_4: assert property (@(posedge clk) disable iff (rst) ov_valid_3 |=> ov_valid_4)
    else $error("AddChain: an instruction leaving stage 'step_2' did not reach stage 'step_3'");
  ov_a_advance_5: assert property (@(posedge clk) disable iff (rst) ov_valid_4 |=> ov_valid_5)
    else $error("AddChain: an instruction leaving stage 'step_3' did not reach stage 'last'");
  ov_a_valid_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({ov_valid_1, ov_valid_2, ov_valid_3, ov_valid_4, ov_valid_5}))
    else $error("AddChain: a valid bit is X (missing reset?)");
  `endif
endmodule
