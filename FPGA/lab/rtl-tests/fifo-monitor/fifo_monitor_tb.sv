`timescale 1ns/1ps

module fifo_monitor_tb;
  reg clk = 0, resetn = 0;
  reg [15:0] araddr = 0;
  reg arvalid = 0, rready = 0;
  wire arready, rvalid;
  wire [31:0] rdata;
  reg [15:0] awaddr = 0;
  reg awvalid = 0, wvalid = 0, bready = 0;
  reg [31:0] wdata = 0;
  wire awready, wready, bvalid;
  wire [1:0] rresp, bresp;
  reg [31:0] c1 = 0, c2 = 0, c3 = 0, c4 = 0;
  reg o1 = 0, o2 = 0, o3 = 0, o4 = 0;
  wire i1, i2, i3, i4;

  always #5 clk = ~clk;

  FIFO_Monitor #(.FIFO1_DEPTH(16), .FIFO2_DEPTH(8), .FIFO3_DEPTH(4),
                 .FIFO4_DEPTH(2), .BUILD_ID(32'h5632_3901)) dut (
    .aclk(clk), .aresetn(resetn),
    .s_axi_awaddr(awaddr), .s_axi_awvalid(awvalid), .s_axi_awready(awready),
    .s_axi_wdata(wdata), .s_axi_wvalid(wvalid), .s_axi_wready(wready),
    .s_axi_bresp(bresp), .s_axi_bvalid(bvalid), .s_axi_bready(bready),
    .s_axi_araddr(araddr), .s_axi_arvalid(arvalid), .s_axi_arready(arready),
    .s_axi_rdata(rdata), .s_axi_rresp(rresp), .s_axi_rvalid(rvalid), .s_axi_rready(rready),
    .fifo1_count(c1), .fifo1_overflow(o1), .fifo2_count(c2), .fifo2_overflow(o2),
    .fifo3_count(c3), .fifo3_overflow(o3), .fifo4_count(c4), .fifo4_overflow(o4),
    .int1_out(i1), .int2_out(i2), .int3_out(i3), .int4_out(i4));

  task automatic axi_write(input [15:0] addr, input [31:0] value);
    begin
      @(posedge clk); awaddr <= addr; wdata <= value; awvalid <= 1; wvalid <= 1;
      while (!(awready && wready)) @(posedge clk);
      @(posedge clk); awvalid <= 0; wvalid <= 0; bready <= 1;
      while (!bvalid) @(posedge clk);
      @(posedge clk); bready <= 0;
    end
  endtask

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

  reg [31:0] value, event_value, held_value;
  initial begin
    repeat (4) @(posedge clk);
    resetn <= 1;
    repeat (3) @(posedge clk);

    // Establish extrema, then clear them and capture a coherent count set.
    c1 <= 3; c2 <= 2; c3 <= 1; c4 <= 1;
    repeat (3) @(posedge clk);
    axi_write(16'h20, 32'h2); // clear extrema/event counters
    c1 <= 11; c2 <= 6; c3 <= 3; c4 <= 2;
    repeat (2) @(posedge clk);
    axi_write(16'h20, 32'h1); // atomic snapshot
    axi_read(16'h20, value);
    if (value[31] !== 1'b1 || value[15:0] !== 16'd1) $fatal(1, "snapshot status %h", value);
    axi_read(16'h24, value); if (value !== 32'd11) $fatal(1, "snapshot c1 %h", value);
    axi_read(16'h28, value); if (value !== 32'd6)  $fatal(1, "snapshot c2 %h", value);
    axi_read(16'h2c, value); if (value !== 32'd3)  $fatal(1, "snapshot c3 %h", value);
    axi_read(16'h30, value); if (value !== 32'd2)  $fatal(1, "snapshot c4 %h", value);

    // Exercise full/empty transitions and verify extrema/event telemetry.
    c1 <= 16; c2 <= 8; c3 <= 4; c4 <= 0;
    repeat (2) @(posedge clk);
    c1 <= 0; c2 <= 0; c3 <= 0; c4 <= 2;
    repeat (2) @(posedge clk);
    axi_read(16'h44, value); if (value !== 32'd16) $fatal(1, "max c1 %h", value);
    axi_read(16'h34, value); if (value !== 32'd0)  $fatal(1, "min c1 %h", value);
    axi_read(16'h54, event_value); if (event_value == 0) $fatal(1, "event c1 did not increment");
    axi_read(16'h64, value); if (value !== 32'h5632_3901) $fatal(1, "build id %h", value);

    // AXI4-Lite requires RDATA and RVALID to remain stable until RREADY.
    // Read the live status, then vary its sources for several stalled cycles.
    c1 <= 5;
    repeat (2) @(posedge clk);
    @(posedge clk); araddr <= 16'h00; arvalid <= 1;
    @(posedge clk); arvalid <= 0;
    while (!rvalid) @(posedge clk);
    held_value = rdata;
    c1 <= 16; o1 <= 1;
    repeat (4) begin
      @(posedge clk);
      if (!rvalid || rdata !== held_value)
        $fatal(1, "stalled FIFO response changed: held=%h now=%h", held_value, rdata);
    end
    rready <= 1;
    @(posedge clk); rready <= 0; o1 <= 0;

    // The full condition on the read-completion edge must survive the
    // read-to-clear boundary and appear in the next status response.
    repeat (2) @(posedge clk);
    axi_read(16'h00, value);
    if (value[31] !== 1'b1)
      $fatal(1, "simultaneous full/read-clear event was lost: %h", value);

    $display("SATURN_FIFO_TELEMETRY_OK seq=1 max1=16 events1=%0d", event_value);
    $finish;
  end
endmodule
