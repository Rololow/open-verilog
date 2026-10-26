# Instruction sets

An `isa` block describes the instruction formats once. The compiler uses it for decoding views in the hardware, and the
Python reference can also write from it a SystemVerilog package of field positions, a Python encoder and decoder, and
the documentation.

```ov
const N_BANKS = 4, BANK_ADDR_L = 4, DATA_MEM_ADDR_L = 10;

isa Einsum "toy accelerator" : opcode 4 max 128 {
  NOP = 1 "no operation" {}
  LD = 5 "load" { en : N_BANKS "one enable per bank"; mem_addr : DATA_MEM_ADDR_L "data memory address"; }
  LD_SC = 12 "load from the spill memory" like LD;
  BB = 2 "compute" { pe_mode : 4; rd_addr : BANK_ADDR_L [N_BANKS] "read address per bank"; }
}

module Decoder(input logic [63:0] instr, input logic [1:0] bank, output logic [3:0] op, output logic [9:0] addr,
               output logic [3:0] en, output logic [3:0] rd2, output logic [3:0] rdv, output logic [7:0] len) {
  val ld = instr.as(Einsum.LD);
  val bb = instr.as(Einsum.BB);
  assign op = ld.opcode;
  assign addr = ld.mem_addr;
  assign en = ld.en;
  assign rd2 = bb.rd_addr[2];
  assign rdv = bb.rd_addr[bank];
  assign len = Einsum.lengths[op];
}
```

## The parts

- `opcode 4`: the opcode field is 4 bits wide. The generated package gives the longest instruction of this set as 24 bits.
- Each instruction has an opcode (`NOP = 1`), a description, and fields. A field is a width (`pe_mode : 4`), or a width
  per element of an array (`rd_addr : BANK_ADDR_L [N_BANKS]`).
- `LD_SC ... like LD`: the same fields as `LD`, another opcode.
- `instr.as(Einsum.LD)`: a view of the bits of `instr` as the `LD` format. Its fields are read by name, and `.opcode`
  gives the opcode.
- `Einsum.lengths[op]`: the length in bits of the instruction whose opcode is `op`.

## What the compiler writes from the same block

`ovc isa.ov` with these options writes three files from the same definition:

- `--emit-isa-sv FILE`: a SystemVerilog package with the position and length of every field, and each opcode;
- `--emit-isa-py FILE`: a Python encoder and decoder, to write programs for the hardware and read its traces;
- `--emit-doc FILE`: the documentation of the formats.

The package and the decoder are generated from the same definition, so they cannot disagree. The Rust compiler writes
the same three files as the Python reference (see [Command line](Command-Line)).
