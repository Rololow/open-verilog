module Decoder (
  input logic [63:0] instr,
  input logic [1:0] bank,
  output logic [3:0] op,
  output logic [9:0] addr,
  output logic [3:0] en,
  output logic [3:0] rd2,
  output logic [3:0] rdv,
  output logic [7:0] len
);
  localparam logic [4:0] Einsum_lengths [16] = '{4, 4, 24, 4, 4, 18, 4, 4, 4, 4, 4, 4, 18, 4, 4, 4};
  assign op = instr[3:0];
  assign addr = instr[17:8];
  assign en = instr[7:4];
  assign rd2 = instr[19:16];
  assign rdv = instr[8 + bank * 4+:4];
  assign len = Einsum_lengths[op];
endmodule
