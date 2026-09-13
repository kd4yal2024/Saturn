`timescale 1ns/1ps

module adc_fifo_reader_tb;
  reg clk = 0, resetn = 0;
  reg [15:0] araddr = 0;
  reg arvalid = 0, rready = 0;
  wire arready, rvalid;
  wire [31:0] rdata;
  wire [1:0] rresp;
  reg [15:0] adc1 = 0, adc2 = 0;
  reg [15:0] overflows = 0;
  wire [1:0] bresp;
  wire awready, wready, bvalid;

  always #4 clk = ~clk;

  AXI_FIFO_overflow_reader dut (
    .aclk(clk), .aresetn(resetn),
    .s_axi_awaddr(16'd0), .s_axi_awvalid(1'b0), .s_axi_awready(awready),
    .s_axi_wdata(32'd0), .s_axi_wvalid(1'b0), .s_axi_wready(wready),
    .s_axi_bresp(bresp), .s_axi_bvalid(bvalid), .s_axi_bready(1'b0),
    .s_axi_araddr(araddr), .s_axi_arvalid(arvalid), .s_axi_arready(arready),
    .s_axi_rdata(rdata), .s_axi_rresp(rresp), .s_axi_rvalid(rvalid), .s_axi_rready(rready),
    .overflow1(overflows[0]), .overflow2(overflows[1]), .overflow3(overflows[2]),
    .overflow4(overflows[3]), .overflow5(overflows[4]), .overflow6(overflows[5]),
    .overflow7(overflows[6]), .overflow8(overflows[7]), .overflow9(overflows[8]),
    .overflow10(overflows[9]), .overflow11(overflows[10]), .overflow12(overflows[11]),
    .overflow13(overflows[12]), .overflow14(overflows[13]), .overflow15(overflows[14]),
    .overflow16(overflows[15]), .ADC1data(adc1), .ADC2data(adc2));

  task automatic axi_read(input [15:0] addr, output [31:0] value);
    begin
      @(posedge clk); araddr <= addr; arvalid <= 1;
      @(posedge clk); arvalid <= 0;
      while (!rvalid) @(posedge clk);
      value = rdata;
      @(posedge clk); rready <= 1;
      @(posedge clk); rready <= 0;
    end
  endtask

  reg [31:0] value, held_value;
  integer code;
  integer expected_magnitude;
  initial begin
    repeat (4) @(posedge clk);
    resetn <= 1;

    // Exhaust every signed 16-bit ADC code, including -32768.  Inspect the
    // registered magnitude stage directly so one preceding maximum cannot
    // mask a bad conversion later in the sweep.
    for (code = 0; code < 65536; code = code + 1) begin
      @(negedge clk); adc1 = code[15:0];
      repeat (2) @(posedge clk);
      #1;
      expected_magnitude = code[15] ? (65536 - code) : code;
      if (dut.ADC1magnitudereg !== expected_magnitude[16:0])
        $fatal(1, "ADC magnitude code=%h got=%0d expected=%0d",
               code[15:0], dut.ADC1magnitudereg, expected_magnitude);
    end

    // Reset the measurement window after the exhaustive conversion check.
    @(negedge clk); resetn = 0; adc1 = 0; adc2 = 0;
    repeat (3) @(posedge clk);
    @(negedge clk); resetn = 1;
    adc1 <= 16'h8000; adc2 <= 16'h7fff; overflows[0] <= 1'b1;
    repeat (5) @(posedge clk);
    overflows[0] <= 1'b0; adc1 <= 16'd0; adc2 <= 16'd0;
    repeat (3) @(posedge clk);

    // A peak-only read must not clear the pending overflow indication.
    axi_read(16'h04, value);

    // Reading overflow address creates one coherent ADC/overflow snapshot.
    axi_read(16'h00, value);
    if (value[0] !== 1'b1) $fatal(1, "overflow snapshot %h", value);
    axi_read(16'h10, value);
    if (value[31] !== 1'b1 || value[15:0] !== 16'd1) $fatal(1, "snapshot status %h", value);
    axi_read(16'h14, value);
    if (value !== 32'd32768) $fatal(1, "ADC1 17-bit peak %h", value);
    axi_read(16'h18, value);
    if (value !== 32'd32767) $fatal(1, "ADC2 17-bit peak %h", value);
    axi_read(16'h1c, value);
    if (value[0] !== 1'b1) $fatal(1, "latched overflow %h", value);

    // Hold the response stalled while live telemetry changes.  RDATA/RVALID
    // must remain stable, and an event after the accepted address must be
    // retained for the next snapshot window.
    @(posedge clk); araddr <= 16'h00; arvalid <= 1;
    @(posedge clk); arvalid <= 0;
    while (!rvalid) @(posedge clk);
    held_value = rdata;
    overflows[1] <= 1'b1; adc1 <= 16'h4000;
    repeat (4) begin
      @(posedge clk);
      if (!rvalid || rdata !== held_value)
        $fatal(1, "stalled ADC response changed: held=%h now=%h", held_value, rdata);
    end
    rready <= 1;
    @(posedge clk); rready <= 0; overflows[1] <= 1'b0;
    repeat (3) @(posedge clk);
    axi_read(16'h00, value);
    if (value[1] !== 1'b1)
      $fatal(1, "event after snapshot boundary was lost: %h", value);

    $display("SATURN_ADC_TELEMETRY_OK adc1=32768 adc2=32767");
    $finish;
  end
endmodule
