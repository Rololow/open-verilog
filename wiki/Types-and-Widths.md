# Types and widths

## Widths

One rule decides what a number in a declaration means:

- dimensions **before** the name are bit widths: `logic [7:0] a`, `logic [8] a`, `logic<8> a` are all 8 bits;
- dimensions **after** the name, or `Vec<T, N>`, are array sizes: `logic [7:0] a [3]` and `Vec<logic<8>, 3> a` are
  three elements of 8 bits.

```ov
module Widths(
  input  logic<8> a,                   // 8 bits, same as logic [7:0]
  input  logic [8] b,                  // 8 bits (shorthand: [N] before the name means N bits)
  input  Vec<logic<8>, 3> arr,         // 3 elements of 8 bits, same as logic [7:0] arr [3]
  output logic<11> sum
) {
  assign sum = a + b + arr[0] + arr[1] + arr[2];
}
```

## Packages, typedefs and packed structs

SystemVerilog-style packages work as they do in SystemVerilog, and `.ov` code can use the same types. A packed struct is
a bit vector: `p.field` is a part-select, and the first field is the most significant bits.

```ov
package Defs;
  typedef logic [7:0] byte_t;
  typedef struct packed {
    logic [3:0] kind;
    byte_t payload;
    logic parity;
  } pkt_t;
  typedef enum logic [1:0] { RED, GREEN, BLUE } color_t;
  localparam int PKT_W = 13;
  function automatic logic parity_of(input logic [7:0] v);
    return ^v;
  endfunction
endpackage

module Pack import Defs::*; (input byte_t data, input logic [3:0] kind, output pkt_t pkt, output color_t c);
  assign pkt.kind = kind;
  assign pkt.payload = data;
  assign pkt.parity = parity_of(data);
  assign c = data[0] ? GREEN : RED;
endmodule

// .ov style: no typedef needed
struct Pair { logic [7:0] hi; logic [7:0] lo; }

module Swap(input Pair p, output Pair q) {
  assign q.hi = p.lo;
  assign q.lo = p.hi;
}
```

## Structs: parameterized and nested

A struct can take parameters, and a field can be a struct. Each struct is a packed SystemVerilog type; its package names
the types it uses. A parameterized struct gets one type per instance.

```ov
struct Pix { logic [7:0] r; logic [7:0] g; }

struct Box<int W> { logic [W-1:0] v; logic [W-1:0] w; }

struct Pair { Box<4> a; Box<4> b; }

module Sum(input Vec<Pix, 4> px, output logic [8:0] s) {
  assign s = px[2].r + px[3].g;
}

module Mix(input Pair p, output logic [3:0] q) {
  assign q = p.a.v ^ p.b.w;
}

module Wide(input Box<8> b, output logic [7:0] y) {
  assign y = b.v & b.w;
}
```

Members of a nested struct are read and written as `p.inner.x`:

```ov
struct Point { logic [7:0] x; logic [7:0] y; }

struct Segment { Point from; Point to; logic [3:0] tag; }

module Span(input Segment s, output logic [8:0] dx, output logic [3:0] tag) {
  assign dx = s.to.x - s.from.x;
  assign tag = s.tag;
}

module Start(input logic [7:0] x, input logic [7:0] y, output Segment s) {
  assign s.from.x = x;
  assign s.from.y = y;
  assign s.to = s.from;
  assign s.tag = 3;
}
```

## Mixing with SystemVerilog

A file can mix classic SystemVerilog modules, with `generate` and `genvar`, and `.ov` modules. Nothing has to be
converted first.

```ov
module Reg8 #(parameter int W = 8) (
  input  logic clk,
  input  logic [W-1:0] d,
  output logic [W-1:0] q
);
  always_ff @(posedge clk) begin
    q <= d;
  end
endmodule

module Bank #(parameter int N = 4, parameter int W = 8) (
  input  logic clk,
  input  logic [W-1:0] d [N],
  output logic [W-1:0] q [N]
);
  genvar i;
  generate
    for (genvar i = 0; i < N; i++) begin : g
      Reg8 #(.W(W)) r (.clk, .d(d[i]), .q(q[i]));
    end
  endgenerate
endmodule

module Acc<int W = 8>(input clk, input rst, input logic [W-1:0] x, output logic [W-1:0] s) {
  always @(posedge clk or negedge rst) begin
    if (!rst) s <= 0;
    else s += x;
  end
}
```

`Acc` is written in the `.ov` style for its parameters and in the SystemVerilog style for its `always` block: both are
accepted.
