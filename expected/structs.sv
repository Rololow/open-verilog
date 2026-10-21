package pix_pkg;
  typedef struct packed {
    logic [7:0] r;
    logic [7:0] g;
  } pix_t;
endpackage

package box_4_pkg;
  typedef struct packed {
    logic [3:0] v;
    logic [3:0] w;
  } box_4_t;
endpackage

package pair_pkg;
  typedef struct packed {
    box_4_pkg::box_4_t a;
    box_4_pkg::box_4_t b;
  } pair_t;
endpackage

package box_8_pkg;
  typedef struct packed {
    logic [7:0] v;
    logic [7:0] w;
  } box_8_t;
endpackage

module Sum (
  input pix_pkg::pix_t px [4],
  output logic [8:0] s
);
  assign s = px[2].r + px[3].g;
endmodule

module Mix (
  input pair_pkg::pair_t p,
  output logic [3:0] q
);
  assign q = p.a.v ^ p.b.w;
endmodule

module Wide (
  input box_8_pkg::box_8_t b,
  output logic [7:0] y
);
  assign y = b.v & b.w;
endmodule
