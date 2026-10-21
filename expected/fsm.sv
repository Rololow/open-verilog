module Handshake (
  input logic clk,
  input logic rst,
  input logic req,
  output logic ack,
  output logic busy
);
  localparam logic [1:0] State_IDLE = 0;
  localparam logic [1:0] State_BUSY = 1;
  localparam logic [1:0] State_DONE = 2;
  logic [1:0] state = State_IDLE;
  logic [1:0] state_next;
  always_comb begin
    state_next = state;
    ack = 0;
    busy = 0;
    case (state)
      State_IDLE: begin
        if (req) begin
          state_next = State_BUSY;
        end
      end
      State_BUSY: begin
        busy = 1;
        state_next = State_DONE;
      end
      State_DONE: begin
        ack = 1;
        state_next = State_IDLE;
      end
    endcase
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      state <= State_IDLE;
    end else begin
      state <= state_next;
    end
  end
endmodule

module Alu (
  input logic [1:0] op,
  input logic [7:0] a,
  input logic [7:0] b,
  output logic [7:0] y
);
  localparam logic [1:0] Op_ADD = 0;
  localparam logic [1:0] Op_SUB = 1;
  localparam logic [1:0] Op_AND = 3;
  always_comb begin
    case (op)
      Op_ADD: begin
        y = a + b;
      end
      Op_SUB: begin
        y = a - b;
      end
      Op_AND: begin
        y = a & b;
      end
      default: begin
        y = 0;
      end
    endcase
  end
endmodule
