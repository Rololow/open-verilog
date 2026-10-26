module Cell__8 (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  always_ff @(posedge clk) begin
    q <= d;
  end
endmodule

module Shift (
  input logic clk,
  input logic [7:0] d,
  output logic [7:0] q
);
  logic [7:0] stage [5];
  assign stage[0] = d;
  for (genvar i = 0; i < 4; i++) begin : gen0
    Cell__8 c (.clk(clk), .d(stage[i]), .q(stage[i + 1]));
  end
  assign q = stage[4];
endmodule

module Parity (
  input logic [7:0] x,
  output logic p
);
  always_comb begin
    p = 0;
    for (int i = 0; i < 8; i++) begin
      p = p ^ x[i];
    end
  end
endmodule

module Sel (
  input logic [1:0] s,
  input logic [3:0] a,
  output logic y
);
  always_comb begin
    unique0 case (s)
      0, 1: begin
        y = a[0];
      end
      2: begin
        y = a[1] & a[2] | a[3];
      end
      default: begin
        y = (a[0] + a[1]) * 2 == 0 ? 1 : 0;
      end
    endcase
  end
endmodule
