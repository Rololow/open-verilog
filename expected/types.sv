package pkt_t_pkg;
  typedef struct packed {
    logic [3:0] kind;
    logic [7:0] payload;
    logic parity;
  } pkt_t;
endpackage

package pair_pkg;
  typedef struct packed {
    logic [7:0] hi;
    logic [7:0] lo;
  } pair_t;
endpackage

module Pack (
  input logic [7:0] data,
  input logic [3:0] kind,
  output pkt_t_pkg::pkt_t pkt,
  output logic [1:0] c
);
  localparam logic [1:0] color_t_RED = 0;
  localparam logic [1:0] color_t_GREEN = 1;
  localparam logic [1:0] color_t_BLUE = 2;
  assign pkt.kind = kind;
  assign pkt.payload = data;
  assign pkt.parity = ^data;
  assign c = data[0] ? color_t_GREEN : color_t_RED;
endmodule

module Swap (
  input pair_pkg::pair_t p,
  output pair_pkg::pair_t q
);
  assign q.hi = p.lo;
  assign q.lo = p.hi;
endmodule
