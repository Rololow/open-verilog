module Source (
  input logic clk,
  input logic rst,
  output logic out_valid,
  input logic out_ready,
  output logic [7:0] out_data
);
  logic [7:0] n;
  always_ff @(posedge clk) begin
    if (rst) begin
      n <= 0;
    end else begin
      if (out_valid & out_ready) begin
        n <= n + 1;
      end
    end
  end
  assign out_valid = 1;
  assign out_data = n;
  `ifndef SYNTHESIS
  // ovc: valid/ready protocol checks
  ov_hs_out_held: assert property (@(posedge clk) disable iff (rst) out_valid && !out_ready |=> out_valid && $stable(out_data))
    else begin $error("Source: out offered a value and it withdrew or changed it before it was accepted (valid must stay high, with stable data, until ready)"); `ifdef OVC_TESTS ->ovc_tests.ovc_check_failed; `endif end
  ov_hs_out_known: assert property (@(posedge clk) disable iff (rst) !$isunknown(out_valid))
    else begin $error("Source: out valid is X"); `ifdef OVC_TESTS ->ovc_tests.ovc_check_failed; `endif end
  `endif
endmodule

module Sink (
  input logic clk,
  input logic rst,
  input logic go,
  input logic in_valid,
  output logic in_ready,
  input logic [7:0] in_data,
  output logic got,
  output logic [7:0] got_data
);
  assign in_ready = go;
  assign got = in_valid & in_ready;
  assign got_data = in_data;
  `ifndef SYNTHESIS
  // ovc: valid/ready protocol checks
  ov_hs_in_held: assert property (@(posedge clk) disable iff (rst) in_valid && !in_ready |=> in_valid && $stable(in_data))
    else begin $error("Sink: in offered a value and the producer connected to it withdrew or changed it before it was accepted (valid must stay high, with stable data, until ready)"); `ifdef OVC_TESTS ->ovc_tests.ovc_check_failed; `endif end
  ov_hs_in_known: assert property (@(posedge clk) disable iff (rst) !$isunknown(in_valid))
    else begin $error("Sink: in valid is X"); `ifdef OVC_TESTS ->ovc_tests.ovc_check_failed; `endif end
  `endif
endmodule

module Top (
  input logic clk,
  input logic rst,
  input logic go,
  output logic got,
  output logic [7:0] got_data
);
  logic link_valid;
  logic link_ready;
  logic [7:0] link_data;
  Source src (.clk(clk), .rst(rst), .out_valid(link_valid), .out_ready(link_ready), .out_data(link_data));
  Sink snk (.clk(clk), .rst(rst), .go(go), .in_valid(link_valid), .in_ready(link_ready), .in_data(link_data), .got(got), .got_data(got_data));
endmodule
