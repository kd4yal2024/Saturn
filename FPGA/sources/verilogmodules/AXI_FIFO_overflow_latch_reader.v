
`timescale 1 ns / 1 ps
//////////////////////////////////////////////////////////////////////////////////
// Company: HPSDR
// Engineer: Laurence Barker G8NJJ
// 
// Create Date: 17.05.2021 10:24:28
// Design Name: 
// Module Name: AXI_FIFO_overflow_reader
// Project Name: Saturn
// Target Devices: Artix 7
// Tool Versions: 
// Description: 
// latch FIFO overflow indications and hold until read.
// also used for FIFO overflows.
// function added: also record ADC peak samples within the same period as ADC overflows
// AXI4-lite bus interface to read back the overflow indications and clear the latch.
//
// Registers:
//  addr 0         Overflow register (read only, with side effect)
//                 bit 0: reads out latched overflow 1
//                 bit 1: reads out latched overflow 2
//                 bit 15: reads out latched overflow 16
//	An axi4 read transaction clears the latch.
//  on read: the ADC peak values are latched, read yto be read out.
// ** it is critical to read the Overflow register first **
//
// addr 4          ADC1 peak amplitude value (16 bit unsigned)
// addr 8          ADC2 peak amplitude value (16 bit unsigned)
// addr C          ADC2 peak amplitude value (16 bit unsigned, legacy alias)
// addr 10         Snapshot status (bit31 valid, bits15:0 sequence)
// addr 14         ADC1 peak amplitude value (17 bit unsigned)
// addr 18         ADC2 peak amplitude value (17 bit unsigned)
// addr 1C         Coherent overflow snapshot (bits 15:0)
// addr 20         V30 ADC telemetry build ID ("V30\0")
// addr 24         ADC1 boot-lifetime overrange episode count
// addr 28         ADC2 boot-lifetime overrange episode count
// addr 2C         ADC1 boot-lifetime overrange clocks
// addr 30         ADC2 boot-lifetime overrange clocks
// addr 34         ADC1 longest continuous overrange episode, clocks
// addr 38         ADC2 longest continuous overrange episode, clocks
// addr 3C         ADC1 latest/current overrange episode length, clocks
// addr 40         ADC2 latest/current overrange episode length, clocks
// addr 44         ADC1 latest/current overrange episode peak (17 bit)
// addr 48         ADC2 latest/current overrange episode peak (17 bit)
// addr 4C         Episode state (bits 1:0 active, bits 9:8 valid)
// addr 50         Episode clock frequency, Hz (122880000)
//
// Registers 24..4C are captured coherently at the same accepted addr 0 read
// boundary as the legacy overflow and peak snapshot. Counters saturate and
// clear only on FPGA reset. An episode is one sampled low-to-high transition
// followed by all consecutive high clocks; repeated status reads cannot
// create additional episodes.


//
// Dependencies: 
// 
// Revision:
// Revision 0.01 - File Created
// Additional Comments: 
// 
//////////////////////////////////////////////////////////////////////////////////

module AXI_FIFO_overflow_reader #
(
  parameter integer AXI_DATA_WIDTH = 32,
  parameter integer AXI_ADDR_WIDTH = 16
)
(
  // System signals
  input  wire                      aclk,
  input  wire                      aresetn,

  // AXI bus Slave 
  input  wire [AXI_ADDR_WIDTH-1:0] s_axi_awaddr,  // AXI4-Lite slave: Write address
  input  wire                      s_axi_awvalid, // AXI4-Lite slave: Write address valid
  output wire                      s_axi_awready, // AXI4-Lite slave: Write address ready
  input  wire [AXI_DATA_WIDTH-1:0] s_axi_wdata,   // AXI4-Lite slave: Write data
  input  wire                      s_axi_wvalid,  // AXI4-Lite slave: Write data valid
  output wire                      s_axi_wready,  // AXI4-Lite slave: Write data ready
  output wire [1:0]                s_axi_bresp,   // AXI4-Lite slave: Write response
  output wire                      s_axi_bvalid,  // AXI4-Lite slave: Write response valid
  input  wire                      s_axi_bready,  // AXI4-Lite slave: Write response ready
  input  wire [AXI_ADDR_WIDTH-1:0] s_axi_araddr,  // AXI4-Lite slave: Read address
  input  wire                      s_axi_arvalid, // AXI4-Lite slave: Read address valid
  output wire                      s_axi_arready, // AXI4-Lite slave: Read address ready
  output wire [AXI_DATA_WIDTH-1:0] s_axi_rdata,   // AXI4-Lite slave: Read data
  output wire [1:0]                s_axi_rresp,   // AXI4-Lite slave: Read data response
  output wire                      s_axi_rvalid,  // AXI4-Lite slave: Read data valid
  input  wire                      s_axi_rready,  // AXI4-Lite slave: Read data ready


// FIFO overflow signals
    input wire overflow1,				// FIFO1 overflow input
    input wire overflow2,				// FIFO2 overflow input
    input wire overflow3,				// FIFO3 overflow input
    input wire overflow4,				// FIFO4 overflow input
    input wire overflow5,				// FIFO5 overflow input
    input wire overflow6,				// FIFO6 overflow input
    input wire overflow7,				// FIFO7 overflow input
    input wire overflow8,				// FIFO8 overflow input
    input wire overflow9,				// FIFO9 overflow input
    input wire overflow10,				// FIFO10 overflow input
    input wire overflow11,				// FIFO11 overflow input
    input wire overflow12,				// FIFO12 overflow input
    input wire overflow13,				// FIFO13 overflow input
    input wire overflow14,				// FIFO14 overflow input
    input wire overflow15,				// FIFO15 overflow input
    input wire overflow16,				// FIFO16 overflow input
    
// ADC input data for ADC max sample value detection
    input wire [15:0] ADC1data,
    input wire [15:0] ADC2data
);

  reg [AXI_DATA_WIDTH-1:0] raddrreg;
  reg [AXI_DATA_WIDTH-1:0] rdatareg;
  reg [AXI_DATA_WIDTH-1:0] overflowdatareg;
  reg [AXI_DATA_WIDTH-1:0] overflowdataregpl1;      // pipelined once
  reg [AXI_DATA_WIDTH-1:0] overflowdataregpl2;      // pipelined twice
  reg [AXI_DATA_WIDTH-1:0] overflowsnapshotreg;
  reg signed [15:0]        ADC1datareg;
  reg signed [15:0]        ADC2datareg;
  reg [16:0]               ADC1magnitudereg;
  reg [16:0]               ADC2magnitudereg;
  reg [AXI_DATA_WIDTH-1:0] ADC1latchedpeakreg;
  reg [AXI_DATA_WIDTH-1:0] ADC2latchedpeakreg;
  reg [16:0]               ADC1currentpeakreg;
  reg [16:0]               ADC2currentpeakreg;
  reg [16:0]               ADC1snapshotpeakreg;
  reg [16:0]               ADC2snapshotpeakreg;
  reg [15:0]               snapshot_sequence;
  reg                      snapshot_valid;
  reg                      ADC1overflowprev;
  reg                      ADC2overflowprev;
  reg                      ADC1episodevalid;
  reg                      ADC2episodevalid;
  reg [31:0]               ADC1episodecountreg;
  reg [31:0]               ADC2episodecountreg;
  reg [31:0]               ADC1totalhighreg;
  reg [31:0]               ADC2totalhighreg;
  reg [31:0]               ADC1currentrunreg;
  reg [31:0]               ADC2currentrunreg;
  reg [31:0]               ADC1longestrunreg;
  reg [31:0]               ADC2longestrunreg;
  reg [31:0]               ADC1latestrunreg;
  reg [31:0]               ADC2latestrunreg;
  reg [16:0]               ADC1episodepeakreg;
  reg [16:0]               ADC2episodepeakreg;
  reg [16:0]               ADC1latestepisodepeakreg;
  reg [16:0]               ADC2latestepisodepeakreg;
  reg [31:0]               ADC1episodecountsnapshotreg;
  reg [31:0]               ADC2episodecountsnapshotreg;
  reg [31:0]               ADC1totalhighsnapshotreg;
  reg [31:0]               ADC2totalhighsnapshotreg;
  reg [31:0]               ADC1longestrunsnapshotreg;
  reg [31:0]               ADC2longestrunsnapshotreg;
  reg [31:0]               ADC1latestrunsnapshotreg;
  reg [31:0]               ADC2latestrunsnapshotreg;
  reg [16:0]               ADC1latestpeaksnapshotreg;
  reg [16:0]               ADC2latestpeaksnapshotreg;
  reg [9:0]                ADCepisodestatesnapshotreg;
  reg arreadyreg;                           // false when write address has been latched
  reg rvalidreg;                            // true when read data out is valid

  wire [15:0] overflow_inputs = {
    overflow16, overflow15, overflow14, overflow13,
    overflow12, overflow11, overflow10, overflow9,
    overflow8, overflow7, overflow6, overflow5,
    overflow4, overflow3, overflow2, overflow1
  };

  function [31:0] saturating_increment;
    input [31:0] value;
    begin
      saturating_increment = (&value) ? value : value + 32'd1;
    end
  endfunction

  wire [16:0] ADC1inputmagnitude = ADC1data[15]
    ? {1'b0, (~ADC1data + 16'd1)} : {1'b0, ADC1data};
  wire [16:0] ADC2inputmagnitude = ADC2data[15]
    ? {1'b0, (~ADC2data + 16'd1)} : {1'b0, ADC2data};
  wire ADC1episoderising = overflow1 & ~ADC1overflowprev;
  wire ADC2episoderising = overflow2 & ~ADC2overflowprev;
  wire [31:0] ADC1episoderunnext = overflow1
    ? (ADC1episoderising ? 32'd1 : saturating_increment(ADC1currentrunreg)) : 32'd0;
  wire [31:0] ADC2episoderunnext = overflow2
    ? (ADC2episoderising ? 32'd1 : saturating_increment(ADC2currentrunreg)) : 32'd0;
  wire [16:0] ADC1episodepeaknext = ADC1episoderising
    ? ADC1inputmagnitude
    : ((ADC1inputmagnitude > ADC1episodepeakreg) ? ADC1inputmagnitude : ADC1episodepeakreg);
  wire [16:0] ADC2episodepeaknext = ADC2episoderising
    ? ADC2inputmagnitude
    : ((ADC2inputmagnitude > ADC2episodepeakreg) ? ADC2inputmagnitude : ADC2episodepeakreg);
  wire [31:0] ADC1episodecountnext = ADC1episoderising
    ? saturating_increment(ADC1episodecountreg) : ADC1episodecountreg;
  wire [31:0] ADC2episodecountnext = ADC2episoderising
    ? saturating_increment(ADC2episodecountreg) : ADC2episodecountreg;
  wire [31:0] ADC1totalhighnext = overflow1
    ? saturating_increment(ADC1totalhighreg) : ADC1totalhighreg;
  wire [31:0] ADC2totalhighnext = overflow2
    ? saturating_increment(ADC2totalhighreg) : ADC2totalhighreg;
  wire [31:0] ADC1longestrunnext = (ADC1episoderunnext > ADC1longestrunreg)
    ? ADC1episoderunnext : ADC1longestrunreg;
  wire [31:0] ADC2longestrunnext = (ADC2episoderunnext > ADC2longestrunreg)
    ? ADC2episoderunnext : ADC2longestrunreg;
  wire [31:0] ADC1latestobservablelength = overflow1
    ? ADC1episoderunnext
    : ((ADC1overflowprev & ~overflow1) ? ADC1currentrunreg : ADC1latestrunreg);
  wire [31:0] ADC2latestobservablelength = overflow2
    ? ADC2episoderunnext
    : ((ADC2overflowprev & ~overflow2) ? ADC2currentrunreg : ADC2latestrunreg);
  wire [16:0] ADC1latestobservablepeak = overflow1
    ? ADC1episodepeaknext
    : ((ADC1overflowprev & ~overflow1) ? ADC1episodepeakreg : ADC1latestepisodepeakreg);
  wire [16:0] ADC2latestobservablepeak = overflow2
    ? ADC2episodepeaknext
    : ((ADC2overflowprev & ~overflow2) ? ADC2episodepeakreg : ADC2latestepisodepeakreg);

//
// AXI read strategy:
// 1. at reset, assert arready and tready, to be able to accept address and stream transfers 
// 1a. latch the overrrange bits when they occur
// 2. when arvalid is true, signalling address transfer, deassert arready 
// 3. assert rvalid when arvalid is false
// 4. when rvalid and rready both true, data is transferred:
// 4a. clear the data;
// 4b. deassert rvalid
// 4c. reassert arready
// it is a requirement that there is no combinatorial path from inpu tot output
//


  assign s_axi_rdata = rdatareg;
  assign s_axi_arready = arreadyreg;
  assign s_axi_rvalid = rvalidreg;
  assign s_axi_rresp = 2'd0;
//
// and outputs to make sure we don't respond to a write
//
  assign s_axi_bresp = 2'd0;                         // no response to write access
  assign s_axi_awready = 1'b0;                       // no response to write access
  assign s_axi_wready = 1'b0;                        // no response to write access
  assign s_axi_bvalid = 1'b0;                        // no response to write access



  always @(posedge aclk)
  begin
    if(~aresetn)
    begin
// step 1
      raddrreg <= {(AXI_DATA_WIDTH){1'b0}};
      rdatareg <= {(AXI_DATA_WIDTH){1'b0}};

      overflowdatareg <= {(AXI_DATA_WIDTH){1'b0}};
      overflowdataregpl1 <= {(AXI_DATA_WIDTH){1'b0}};
      overflowdataregpl2 <= {(AXI_DATA_WIDTH){1'b0}};
      overflowsnapshotreg <= {(AXI_DATA_WIDTH){1'b0}};
      ADC1datareg <= 0;
      ADC2datareg <= 0;
      ADC1magnitudereg <= 0;
      ADC2magnitudereg <= 0;
      ADC1latchedpeakreg <= {(AXI_DATA_WIDTH){1'b0}};
      ADC2latchedpeakreg <= {(AXI_DATA_WIDTH){1'b0}};
      ADC1currentpeakreg <=0;
      ADC2currentpeakreg <= 0;
      ADC1snapshotpeakreg <= 0;
      ADC2snapshotpeakreg <= 0;
      snapshot_sequence <= 16'd0;
      snapshot_valid <= 1'b0;
      ADC1overflowprev <= 1'b0;
      ADC2overflowprev <= 1'b0;
      ADC1episodevalid <= 1'b0;
      ADC2episodevalid <= 1'b0;
      ADC1episodecountreg <= 0;
      ADC2episodecountreg <= 0;
      ADC1totalhighreg <= 0;
      ADC2totalhighreg <= 0;
      ADC1currentrunreg <= 0;
      ADC2currentrunreg <= 0;
      ADC1longestrunreg <= 0;
      ADC2longestrunreg <= 0;
      ADC1latestrunreg <= 0;
      ADC2latestrunreg <= 0;
      ADC1episodepeakreg <= 0;
      ADC2episodepeakreg <= 0;
      ADC1latestepisodepeakreg <= 0;
      ADC2latestepisodepeakreg <= 0;
      ADC1episodecountsnapshotreg <= 0;
      ADC2episodecountsnapshotreg <= 0;
      ADC1totalhighsnapshotreg <= 0;
      ADC2totalhighsnapshotreg <= 0;
      ADC1longestrunsnapshotreg <= 0;
      ADC2longestrunsnapshotreg <= 0;
      ADC1latestrunsnapshotreg <= 0;
      ADC2latestrunsnapshotreg <= 0;
      ADC1latestpeaksnapshotreg <= 0;
      ADC2latestpeaksnapshotreg <= 0;
      ADCepisodestatesnapshotreg <= 0;
      arreadyreg <= 1'b1;                           // ready for address transfer
      rvalidreg <= 1'b0;                            // not ready to transfer read data
    end
    else
    begin
// step 1b. latch the overflow bits
      if(overflow1)
        overflowdatareg[0] <= 1'b1;            // latch data
      if(overflow2)
        overflowdatareg[1] <= 1'b1;            // latch data
      if(overflow3)
        overflowdatareg[2] <= 1'b1;            // latch data
      if(overflow4)
        overflowdatareg[3] <= 1'b1;            // latch data
      if(overflow5)
        overflowdatareg[4] <= 1'b1;            // latch data
      if(overflow6)
        overflowdatareg[5] <= 1'b1;            // latch data
      if(overflow7)
        overflowdatareg[6] <= 1'b1;            // latch data
      if(overflow8)
        overflowdatareg[7] <= 1'b1;            // latch data
      if(overflow9)
        overflowdatareg[8] <= 1'b1;            // latch data
      if(overflow10)
        overflowdatareg[9] <= 1'b1;            // latch data
      if(overflow11)
        overflowdatareg[10] <= 1'b1;           // latch data
      if(overflow12)
        overflowdatareg[11] <= 1'b1;           // latch data
      if(overflow13)
        overflowdatareg[12] <= 1'b1;           // latch data
      if(overflow14)
        overflowdatareg[13] <= 1'b1;           // latch data
      if(overflow15)
        overflowdatareg[14] <= 1'b1;           // latch data
      if(overflow16)
        overflowdatareg[15] <= 1'b1;           // latch data
// latch input ADC data
      ADC1datareg <= ADC1data;
      ADC2datareg <= ADC2data;
//
// step 1c. process ADC data to find peaks
// this is pipelined into two cycles. Find magnitude; thern running max magnitude. 
// find
      if(ADC1datareg[15])
        ADC1magnitudereg <= {1'b0, (~ADC1datareg + 16'd1)};
      else
        ADC1magnitudereg <= {1'b0, ADC1datareg};
      if(ADC1magnitudereg > ADC1currentpeakreg)
        ADC1currentpeakreg <= ADC1magnitudereg;

      if(ADC2datareg[15])
        ADC2magnitudereg <= {1'b0, (~ADC2datareg + 16'd1)};
      else
        ADC2magnitudereg <= {1'b0, ADC2datareg};
      if(ADC2magnitudereg > ADC2currentpeakreg)
        ADC2currentpeakreg <= ADC2magnitudereg;

// V30 ADC overrange episode telemetry. These counters are independent of the
// legacy read-to-clear latch above and therefore reflect physical sampled
// overrange intervals rather than software polling frequency.
      ADC1overflowprev <= overflow1;
      ADC2overflowprev <= overflow2;

      if (overflow1)
      begin
        ADC1episodecountreg <= ADC1episodecountnext;
        ADC1episodevalid <= 1'b1;
        ADC1totalhighreg <= ADC1totalhighnext;
        ADC1currentrunreg <= ADC1episoderunnext;
        ADC1episodepeakreg <= ADC1episodepeaknext;
        ADC1longestrunreg <= ADC1longestrunnext;
      end
      else
      begin
        if (ADC1overflowprev)
        begin
          ADC1latestrunreg <= ADC1currentrunreg;
          ADC1latestepisodepeakreg <= ADC1episodepeakreg;
        end
        ADC1currentrunreg <= 0;
        ADC1episodepeakreg <= 0;
      end

      if (overflow2)
      begin
        ADC2episodecountreg <= ADC2episodecountnext;
        ADC2episodevalid <= 1'b1;
        ADC2totalhighreg <= ADC2totalhighnext;
        ADC2currentrunreg <= ADC2episoderunnext;
        ADC2episodepeakreg <= ADC2episodepeaknext;
        ADC2longestrunreg <= ADC2longestrunnext;
      end
      else
      begin
        if (ADC2overflowprev)
        begin
          ADC2latestrunreg <= ADC2currentrunreg;
          ADC2latestepisodepeakreg <= ADC2episodepeakreg;
        end
        ADC2currentrunreg <= 0;
        ADC2episodepeakreg <= 0;
      end

//
// step 1d. Register overflow bits to same pipeline depth
//
    overflowdataregpl1 <= overflowdatareg;
    overflowdataregpl2 <= overflowdataregpl1;
    

// step 2. read address transaction: latch address when arvalid and arready both true
//         and deassert arready as the addres transaction is in its last cycle    
      if(s_axi_arvalid & arreadyreg)
      begin
        arreadyreg <= 1'b0;                     // clear when address transaction happens
        raddrreg <= s_axi_araddr;               // latch the required read address
        if (s_axi_araddr[6:2] == 0)
        begin
          // The accepted status address is the exact sample/clear boundary.
          // Events already visible on this edge belong to this snapshot;
          // later events accumulate in the next window even if RREADY stalls.
          overflowsnapshotreg <= overflowdatareg | {{(AXI_DATA_WIDTH-16){1'b0}}, overflow_inputs};
          ADC1snapshotpeakreg <= ADC1currentpeakreg;
          ADC2snapshotpeakreg <= ADC2currentpeakreg;
          ADC1latchedpeakreg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC1currentpeakreg};
          ADC2latchedpeakreg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC2currentpeakreg};
          snapshot_sequence <= snapshot_sequence + 16'd1;
          snapshot_valid <= 1'b1;
          ADC1episodecountsnapshotreg <= ADC1episodecountnext;
          ADC2episodecountsnapshotreg <= ADC2episodecountnext;
          ADC1totalhighsnapshotreg <= ADC1totalhighnext;
          ADC2totalhighsnapshotreg <= ADC2totalhighnext;
          ADC1longestrunsnapshotreg <= ADC1longestrunnext;
          ADC2longestrunsnapshotreg <= ADC2longestrunnext;
          ADC1latestrunsnapshotreg <= ADC1latestobservablelength;
          ADC2latestrunsnapshotreg <= ADC2latestobservablelength;
          ADC1latestpeaksnapshotreg <= ADC1latestobservablepeak;
          ADC2latestpeaksnapshotreg <= ADC2latestobservablepeak;
          ADCepisodestatesnapshotreg <= {
            (ADC2episodevalid | ADC2episoderising),
            (ADC1episodevalid | ADC1episoderising),
            6'b000000,
            overflow2,
            overflow1
          };
          overflowdatareg <= {(AXI_DATA_WIDTH){1'b0}};
          ADC1currentpeakreg <= 0;
          ADC2currentpeakreg <= 0;
        end
      end

// step 3. assert rvalid when address and stream data transfers are ready
      if(!arreadyreg & !rvalidreg)              // latch exactly one response per address
      begin
        rvalidreg <= 1'b1;                                  // signal ready to complete data
        case (raddrreg[6:2])
            0: rdatareg <= overflowsnapshotreg;
            1: rdatareg <= ADC1latchedpeakreg;
            2: rdatareg <= ADC2latchedpeakreg;
            3: rdatareg <= ADC2latchedpeakreg;
            4: rdatareg <= {snapshot_valid, 15'd0, snapshot_sequence};
            5: rdatareg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC1snapshotpeakreg};
            6: rdatareg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC2snapshotpeakreg};
            7: rdatareg <= overflowsnapshotreg;
            8: rdatareg <= 32'h56333000;
            9: rdatareg <= ADC1episodecountsnapshotreg;
           10: rdatareg <= ADC2episodecountsnapshotreg;
           11: rdatareg <= ADC1totalhighsnapshotreg;
           12: rdatareg <= ADC2totalhighsnapshotreg;
           13: rdatareg <= ADC1longestrunsnapshotreg;
           14: rdatareg <= ADC2longestrunsnapshotreg;
           15: rdatareg <= ADC1latestrunsnapshotreg;
           16: rdatareg <= ADC2latestrunsnapshotreg;
           17: rdatareg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC1latestpeaksnapshotreg};
           18: rdatareg <= {{(AXI_DATA_WIDTH-17){1'b0}}, ADC2latestpeaksnapshotreg};
           19: rdatareg <= {{(AXI_DATA_WIDTH-10){1'b0}}, ADCepisodestatesnapshotreg};
           20: rdatareg <= 32'd122880000;
          default: rdatareg <= {(AXI_DATA_WIDTH){1'b0}};
        endcase
      end

// step 4. When rvalid and rready, terminate the transaction & clear data.
      if(rvalidreg & s_axi_rready)
      begin
        rvalidreg <= 1'b0;                                  // deassert rvalid
        arreadyreg <= 1'b1;                                 // ready for new address
      end
    end
  end



endmodule
