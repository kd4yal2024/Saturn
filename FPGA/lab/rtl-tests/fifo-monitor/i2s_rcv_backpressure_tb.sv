`timescale 1ns/100ps

module i2s_rcv_backpressure_tb;
  reg         aclk = 1'b0;
  reg         resetn = 1'b0;
  reg         Brise = 1'b0;
  reg         Bfall = 1'b0;
  reg         LRrise = 1'b0;
  reg         LRfall = 1'b0;
  reg         BCLK = 1'b0;
  reg         LRCLK = 1'b0;
  reg         din = 1'b0;
  reg         mrecv_axis_tready = 1'b0;
  wire [31:0] mrecv_axis_tdata;
  wire        mrecv_axis_tvalid;

  I2S_rcv dut (
    .resetn(resetn),
    .aclk(aclk),
    .Brise(Brise),
    .Bfall(Bfall),
    .LRrise(LRrise),
    .LRfall(LRfall),
    .mrecv_axis_tdata(mrecv_axis_tdata),
    .mrecv_axis_tvalid(mrecv_axis_tvalid),
    .mrecv_axis_tready(mrecv_axis_tready),
    .BCLK(BCLK),
    .LRCLK(LRCLK),
    .din(din)
  );

  always #5 aclk = ~aclk;

  task automatic inject_stereo_frame(input [15:0] left, input [15:0] right);
    begin
      @(negedge aclk);
      dut.LocalData = left;
      dut.temp_data = right;
      dut.shift_cnt = 18;
      dut.b_clk_cnt = 1;
      LRCLK = 1'b1;
      @(posedge aclk);
      #2;
      LRCLK = 1'b0;
      dut.shift_cnt = 0;
      dut.b_clk_cnt = 0;
    end
  endtask

  task automatic check_condition(input condition, input [255:0] message);
    begin
      if (!condition) begin
        $display("FAIL: %0s", message);
        $fatal(1);
      end
    end
  endtask

  initial begin
    repeat (3) @(posedge aclk);
    resetn = 1'b1;

    inject_stereo_frame(16'ha1a1, 16'h1111);
    check_condition(mrecv_axis_tvalid === 1'b1, "first frame did not assert TVALID");
    check_condition(mrecv_axis_tdata === 32'ha1a11111, "first frame data mismatch");

    // A master must keep both TVALID and TDATA stable until the slave accepts
    // the transfer. A later physical I2S frame must not overwrite this one.
    repeat (3) begin
      @(posedge aclk);
      #2;
      check_condition(mrecv_axis_tvalid === 1'b1, "TVALID fell while TREADY was low");
      check_condition(mrecv_axis_tdata === 32'ha1a11111, "TDATA changed while stalled");
    end

    inject_stereo_frame(16'hb2b2, 16'h2222);
    check_condition(mrecv_axis_tvalid === 1'b1, "TVALID fell on a stalled replacement frame");
    check_condition(mrecv_axis_tdata === 32'ha1a11111,
           "a new I2S frame overwrote an unaccepted AXI-Stream frame");

    @(negedge aclk);
    mrecv_axis_tready = 1'b1;
    @(posedge aclk);
    #2;
    check_condition(mrecv_axis_tvalid === 1'b0, "TVALID did not clear after acceptance");

    $display("PASS: I2S receive AXI-Stream output is stable under backpressure");
    $finish;
  end
endmodule
