# Classes: modules with methods, traits and overrides

A module can have methods, can extend another module, and can declare a trait that its instances must provide. A module
can also be a parameter of another module.

```systemverilog ov
trait Valid {
  fn is_valid() -> logic;
}

module Stage(input clk, input rst, input logic in_v, output logic out_v) : Valid {
  logic v;
  fn is_valid() -> logic { return v; }
  always_ff @(posedge clk) {
    if (rst) { v <= 0; } else { v <= in_v; }
  }
  assign out_v = v;
}

// Same thing, but is_valid() is overridden: valid only while "kill" is low.
module KillableStage(input kill) extends Stage : Valid {
  override fn is_valid() -> logic { return v & ~kill; }
}

module Top<module S : Valid = Stage>(input clk, input rst, input logic a, output logic y) {
  logic mid;
  S s0(.clk, .rst, .in_v(a), .out_v(mid));
  S s1(.clk, .rst, .in_v(mid), .out_v());
  always_comb {
    if (s1.is_valid()) { y = 1; } else { y = s0.is_valid(); }
  }
}
```

## The parts

- `trait Valid`: a list of methods. A module that says `: Valid` must define them.
- `extends Stage`: `KillableStage` has every port, register and method of `Stage`.
- `override fn`: replaces a method of the parent. Here `is_valid()` also depends on `kill`.
- `module S : Valid = Stage`: `Top` takes a module as a parameter. The module must have the trait. Its default is `Stage`.
- `s1.is_valid()`: a method called from outside, on an instance. The call reads the instance's value.

Each instance of `S` is a real instance, so `Top` can be used with `KillableStage` without changing the code of `Top`.
