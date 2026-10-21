module KillableStage (
  input logic clk,
  input logic rst,
  input logic in_v,
  output logic out_v,
  input logic kill
);
  logic v;
  always_ff @(posedge clk) begin
    if (rst) begin
      v <= 0;
    end else begin
      v <= in_v;
    end
  end
  assign out_v = v;
endmodule

module Stage (
  input logic clk,
  input logic rst,
  input logic in_v,
  output logic out_v,
  output logic ov_is_valid
);
  logic v;
  always_ff @(posedge clk) begin
    if (rst) begin
      v <= 0;
    end else begin
      v <= in_v;
    end
  end
  assign out_v = v;
  assign ov_is_valid = v;
endmodule

module Top (
  input logic clk,
  input logic rst,
  input logic a,
  output logic y
);
  logic s1__is_valid;
  logic s0__is_valid;
  logic mid;
  Stage s0 (.clk(clk), .rst(rst), .in_v(a), .out_v(mid), .ov_is_valid(s0__is_valid));
  Stage s1 (.clk(clk), .rst(rst), .in_v(mid), .out_v(), .ov_is_valid(s1__is_valid));
  always_comb begin
    if (s1__is_valid) begin
      y = 1;
    end else begin
      y = s0__is_valid;
    end
  end
endmodule
