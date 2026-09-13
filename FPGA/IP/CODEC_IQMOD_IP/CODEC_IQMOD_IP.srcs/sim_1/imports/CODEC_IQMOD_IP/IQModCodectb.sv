`timescale 1ns / 1ps
//////////////////////////////////////////////////////////////////////////////////
// Company:        HPSDR
// Engineer:       Laurence Barker G8NJJ
// 
// Create Date:    23.07.2021 16:42:01
// Design Name:    CW keyer testbench
// Module Name:    Keyer_Testbench
// Project Name:   Saturn
// Target Devices: Artix 7
// Tool Versions:  Vivado
// Description:    Testbench for CW keyer
// 
// Dependencies: 
// 
// Revision:
// Revision 0.01 - File Created
// Additional Comments:
// 


//Step 2 - Import two required packages: axi_vip_pkg and <component_name>_pkg.
import axi_vip_pkg::*;
import IQCODECBLK_axi_vip_0_0_pkg::*;




module IQModnCodec_tb( );

//////////////////////////////////////////////////////////////////////////////////
// Test Bench Signals
//////////////////////////////////////////////////////////////////////////////////
// Clock and Reset
reg aclk = 0;
reg aclk12=0;
reg aresetn = 1;
reg aresetn12 = 1;
reg[4:0] aclkcntr=0;

reg cw_key_down = 1'b0;
reg TX_ENABLE = 1'b0;
reg protocol_2 = 1'b0;

reg [47:0] TXIQIn_tdata = '0;
reg TXIQIn_tvalid = 1'b0;
wire TXIQIn_tready;

reg Deinterleave = 1'b0;
reg Byteswap = 1'b0;


reg [2:0] Modulation_Setup = '0;
reg IQEnable = 1'b0;
reg Mux_Reset = 1'b0;
reg [31:0] TXTestFreq = '0;
reg TX_Strobe = 1'b0;

//reg [31:0] keyer_config;
// replaced by
reg [7:0] CWPttDelay;
reg [9:0] CWHangTime;
reg [12:0] CWRampLength;
reg CWKeyerEnable;

reg [31:0] CodecConfig;
reg [15:0] SidetoneFreq;
reg [15:0] SidetoneVol;
integer KeyHoldNs;
integer RequiredSampleCount;
integer SampleCount = 0;
integer UnknownSampleCount = 0;
integer NonzeroSampleCount = 0;
integer ChangeCount = 0;
integer BadStepCount = 0;
integer NonzeroQCount = 0;
integer fd_w;
reg CaptureEnabled = 1'b0;
reg [23:0] PreviousI = 24'd0;


wire [47:0] m_axis_TXMod_tdata;
wire m_axis_TXMod_tvalid;
wire m_axis_TXMod_tready;

wire [47:0] m_axis_envelope_tdata;
wire m_axis_envelope_tvalid;
reg m_axis_envelope_tready = 1'b1;
            
wire [15:0] m_axis_sidetoneampl_tdata;
wire m_axis_sidetoneampl_tvalid;

wire CWSampleSelect;
wire cw_ptt;
wire [0:0] TX_OUTPUTENABLE;

//
// clockdivider signals
//
wire TCN;
wire ClockOut;


localparam [31:0] BASE_ADDR = 32'h001C0000;
reg [31:0] addr;
reg [31:0] data;
xil_axi_resp_t 	resp;


//
// instantiate block design. Note name can't be too long 
// or we get pathnames too long for windows.
//

IQCODECBLK_wrapper UUT
   (
    .Byteswap            (Byteswap),
    .CWHangTime          (CWHangTime),
    .CWKeyerEnable       (CWKeyerEnable),
    .CWPttDelay          (CWPttDelay),
    .CWRampLength        (CWRampLength),
    .CWSampleSelect     (CWSampleSelect),
    .Deinterleave        (Deinterleave),
    .IQEnable            (IQEnable),
    .Modulation_Setup    (Modulation_Setup),
    .TXIQIn_tdata        (TXIQIn_tdata),
    .TXIQIn_tready       (TXIQIn_tready),
    .TXIQIn_tvalid       (TXIQIn_tvalid),
    .TXTestFreq          (TXTestFreq),
    .TX_ENABLE           (TX_ENABLE),
    .TX_OUTPUTENABLE     (TX_OUTPUTENABLE),
    .TX_Strobe           (TX_Strobe),
    .aclk                (aclk),
    .aclk12              (aclk12),
    .aresetn             (aresetn),
    .aresetn12           (aresetn12),
    .cw_key_down         (cw_key_down),
    .cw_ptt              (cw_ptt),
    .m_axis_TXMod_tdata         (m_axis_TXMod_tdata),
    .m_axis_TXMod_tvalid        (m_axis_TXMod_tvalid),
    .m_axis_TXMod_tready        (m_axis_TXMod_tready),
    .m_axis_sidetoneampl_tdata  (m_axis_sidetoneampl_tdata),
    .m_axis_sidetoneampl_tvalid (m_axis_sidetoneampl_tvalid),
    .protocol_2                 (protocol_2),
    .Codec_Config               (CodecConfig)
    );
 




//
// instantiate a clock divider to generate TReady
// divide by 640 to get 192KHz for protocol 2 modulation Fs
ClockDivider #(640) Div 
(
    .aclk            (aclk),
    .resetn          (aresetn),
    .ClockOut        (ClockOut),
    .TC              (m_axis_TXMod_tready),
    .TCN             (TCN)
);


 
parameter CLK_PERIOD=8.1380208;              // 122.88MHz
// Generate the clock : 122.88 MHz    
always #(CLK_PERIOD/2) aclk = ~aclk;

// In CW mode (Modulation_Setup == 3), the selected stream is the keying
// envelope: Q is zero and I follows the programmed BRAM ramp.  Capture a
// bounded number of accepted samples and check the numerical contract here so
// a simulation that merely reaches $finish cannot be reported as a pass.
always @(posedge aclk)
begin
    if (CaptureEnabled && m_axis_TXMod_tvalid && m_axis_TXMod_tready &&
        (SampleCount < RequiredSampleCount))
    begin
        if (^m_axis_TXMod_tdata === 1'bx)
            UnknownSampleCount = UnknownSampleCount + 1;
        else
        begin
            $fwrite(fd_w, "%0d,%0d\n", $signed(m_axis_TXMod_tdata[23:0]),
                    $signed(m_axis_TXMod_tdata[47:24]));
            if (m_axis_TXMod_tdata[47:24] != 24'd0)
                NonzeroQCount = NonzeroQCount + 1;
            if (m_axis_TXMod_tdata[23:0] != 24'd0)
                NonzeroSampleCount = NonzeroSampleCount + 1;
            if (SampleCount != 0 && m_axis_TXMod_tdata[23:0] != PreviousI)
            begin
                ChangeCount = ChangeCount + 1;
                if ((m_axis_TXMod_tdata[23:0] < PreviousI) ||
                    ((m_axis_TXMod_tdata[23:0] - PreviousI) != 24'd8192))
                    BadStepCount = BadStepCount + 1;
            end
            PreviousI = m_axis_TXMod_tdata[23:0];
        end
        SampleCount = SampleCount + 1;
    end
end

// create divided by 10 clock
always @(posedge aclk)
begin
    aclkcntr = aclkcntr+1;
    if (aclkcntr >= 5)
    begin
        aclk12 = ~aclk12;
        aclkcntr = 0;
    end
end


//////////////////////////////////////////////////////////////////////////////////
// Main Process
//////////////////////////////////////////////////////////////////////////////////
//
initial begin
    //Assert the reset
    aresetn = 0;
    aresetn12 = 0;
    #340
    // Release the reset
    aresetn = 1;
    aresetn12 = 1;
end

//////////////////////////////////////////////////////////////////////////////////
// The following part controls the AXI VIP. 
//It follows the "Useful Coding Guidelines and Examples" section from PG267
//////////////////////////////////////////////////////////////////////////////////
//
// Step 3 - Declare the agent for the master VIP
IQCODECBLK_axi_vip_0_0_mst_t      master_agent;


initial begin    

KeyHoldNs=20000000;
if($value$plusargs("SATURN_KEY_HOLD_NS=%d", KeyHoldNs))
    $display("CW key hold override = %d ns", KeyHoldNs);
RequiredSampleCount=16;
if($value$plusargs("SATURN_REQUIRED_SAMPLES=%d", RequiredSampleCount))
    $display("Required IQMod sample override = %d", RequiredSampleCount);
fd_w = $fopen("./iqmoddata.txt", "w");
if(!fd_w)
    $fatal(1, "Unable to open iqmoddata.txt");
CWRampLength=3840;
CWHangTime = 10;
CWPttDelay=0;
protocol_2=1;
CWKeyerEnable=1;
Modulation_Setup = 3;
Byteswap = 0;
    SidetoneVol = 24000;					// three quarters amplitude
    SidetoneFreq = 682;						// 500Hz
    CodecConfig = (SidetoneVol << 16) | SidetoneFreq;

//key down after 1us;
// key up after 20ms

// Step 4 - Create a new agent
master_agent = new("master vip agent",UUT.IQCODECBLK_i.axi_vip_0.inst.IF);

// Step 5 - Start the agent
master_agent.start_master();
    
    //Wait for the reset to be released
  wait (aresetn == 1'b1);

//
// the block RAM can't be initialised in block RAM controller mode
// so write it with a simple RAM. This will show up in the simulation plot
//
for(addr=0; addr < 4096; addr=addr+4)
begin
    if(addr <= 3840)
        data=addr * 2048;
    master_agent.AXI4LITE_WRITE_BURST(BASE_ADDR + addr,0,data,resp);
    if(resp != 0)
        $fatal(1, "Ramp BRAM AXI write failed at address 0x%08x: response %0d",
               BASE_ADDR + addr, resp);
end
for(addr=4096; addr < 8192; addr=addr+4)
begin
    master_agent.AXI4LITE_WRITE_BURST(BASE_ADDR + addr,0,8388607,resp);
    if(resp != 0)
        $fatal(1, "Sidetone BRAM AXI write failed at address 0x%08x: response %0d",
               BASE_ADDR + addr, resp);
end




//
// now begin the testbench proper
//
#1000
cw_key_down=1;
CaptureEnabled=1;
#(KeyHoldNs)        // wait to release key
cw_key_down=0;
CaptureEnabled=0;
$fclose(fd_w);
if(SampleCount != RequiredSampleCount)
    $fatal(1, "Captured %0d IQMod samples; expected %0d", SampleCount,
           RequiredSampleCount);
if(UnknownSampleCount != 0)
    $fatal(1, "IQMod output contained %0d samples with unknown bits",
           UnknownSampleCount);
if(NonzeroQCount != 0)
    $fatal(1, "CW IQMod Q output was non-zero in %0d samples", NonzeroQCount);
if(NonzeroSampleCount < (RequiredSampleCount / 2))
    $fatal(1, "CW IQMod I ramp had only %0d non-zero samples",
           NonzeroSampleCount);
if(ChangeCount < (RequiredSampleCount / 2))
    $fatal(1, "CW IQMod I ramp changed only %0d times", ChangeCount);
if(BadStepCount != 0)
    $fatal(1, "CW IQMod I ramp had %0d incorrect numerical steps", BadStepCount);
$display("SATURN_IQMOD_NUMERIC_OK samples=%0d nonzero=%0d changes=%0d step=8192",
         SampleCount, NonzeroSampleCount, ChangeCount);
#1000
$finish;

end
endmodule
