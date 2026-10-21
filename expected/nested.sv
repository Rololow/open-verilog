package point_pkg;
  typedef struct packed {
    logic [7:0] x;
    logic [7:0] y;
  } point_t;
endpackage

package segment_pkg;
  typedef struct packed {
    point_pkg::point_t from;
    point_pkg::point_t to;
    logic [3:0] tag;
  } segment_t;
endpackage

module Span (
  input segment_pkg::segment_t s,
  output logic [8:0] dx,
  output logic [3:0] tag
);
  assign dx = s.to.x - s.from.x;
  assign tag = s.tag;
endmodule

module Start (
  input logic [7:0] x,
  input logic [7:0] y,
  output segment_pkg::segment_t s
);
  assign s.from.x = x;
  assign s.from.y = y;
  assign s.to = s.from;
  assign s.tag = 3;
endmodule
