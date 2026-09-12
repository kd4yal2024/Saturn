# Reviewed Saturn CDC waivers.  Every waiver is bound to exact endpoints so
# hierarchy changes or new destinations fail closed and require fresh review.

proc saturn_lab::require_cdc_endpoints {objects expected description} {
    if {[llength $objects] != $expected} {
        error "CDC waiver endpoint '$description': expected $expected object(s), found [llength $objects]"
    }
    return $objects
}

set pcie_pipe_select [saturn_lab::require_cdc_endpoints \
    [get_pins -quiet {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/gt_top_i/pipe_wrapper_i/pipe_clock_int.pipe_clock_i/pclk_sel_reg/C}] 1 \
    {PCIe PIPE clock select source}]
set pcie_pipe_bufg [saturn_lab::require_cdc_endpoints \
    [get_pins -quiet {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/gt_top_i/pipe_wrapper_i/pipe_clock_int.pipe_clock_i/pclk_i1_bufgctrl.pclk_i1/S1}] 1 \
    {PCIe PIPE BUFGCTRL select}]
create_waiver -type CDC -id CDC-13 -user saturn-fpga \
    -description {Vivado XDMA 4.1 generated PIPE clock-switch circuitry; implementation is owned and qualified by the pinned Xilinx IP} \
    -tags {vendor_ip pcie} -from $pcie_pipe_select -to $pcie_pipe_bufg

set pcie_reset [saturn_lab::require_cdc_endpoints [get_ports -quiet pcie_reset_n] 1 \
    {PCIe fundamental reset port}]
set pcie_reset_fd [saturn_lab::require_cdc_endpoints [get_pins -quiet [list \
    {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/pl_phy_lnk_up_q_reg/R} \
    {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/pl_received_hot_rst_q_reg/R} \
    {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/user_lnk_up_int_reg/R}]] 3 \
    {PCIe generated reset FD endpoints}]
set pcie_reset_pre [saturn_lab::require_cdc_endpoints [get_pins -quiet \
    {saturn_top_i/PCIe/xdma_0/inst/saturn_top_xdma_0_0_pcie2_to_pcie3_wrapper_i/pcie2_ip_i/inst/inst/user_reset_int_reg/PRE}] 1 \
    {PCIe generated reset preset endpoint}]
create_waiver -type CDC -id CDC-1 -user saturn-fpga \
    -description {Asynchronous fundamental reset paths inside pinned Vivado XDMA 4.1 generated logic} \
    -tags {vendor_ip pcie reset} -from $pcie_reset -to $pcie_reset_fd
create_waiver -type CDC -id CDC-7 -user saturn-fpga \
    -description {Asynchronous fundamental reset preset inside pinned Vivado XDMA 4.1 generated logic} \
    -tags {vendor_ip pcie reset} -from $pcie_reset -to $pcie_reset_pre

# clock_monitor deliberately samples each observed clock level in userclk2;
# two successive samples are used to count transitions, not as functional data.
set monitor_clkout [saturn_lab::require_cdc_endpoints [get_pins -quiet \
    {saturn_top_i/clock_generator/clk_wiz_0/inst/mmcm_adv_inst/CLKOUT0}] 1 \
    {clock monitor generated-clock source}]
set monitor_clkout_d [saturn_lab::require_cdc_endpoints [get_pins -quiet [list \
    {saturn_top_i/clock_monitor_0/inst/ck0_rega_reg/D} \
    {saturn_top_i/clock_monitor_0/inst/ck3_rega_reg/D}]] 2 \
    {clock monitor generated-clock samplers}]
create_waiver -type CDC -id CDC-1 -user saturn-fpga \
    -description {Intentional asynchronous level sampling by the diagnostic clock-frequency monitor; not consumed as functional control or data} \
    -tags {diagnostic clock_monitor} -from $monitor_clkout -to $monitor_clkout_d

foreach {port_name destination} {
    EMC_CLK   saturn_top_i/clock_monitor_0/inst/ck2_rega_reg/D
    ref_in_10 saturn_top_i/clock_monitor_0/inst/ck1_rega_reg/D
} {
    set source [saturn_lab::require_cdc_endpoints [get_ports -quiet $port_name] 1 \
        "$port_name clock monitor source"]
    set target [saturn_lab::require_cdc_endpoints [get_pins -quiet $destination] 1 \
        "$port_name clock monitor sampler"]
    create_waiver -type CDC -id CDC-1 -user saturn-fpga \
        -description {Intentional asynchronous level sampling by the diagnostic clock-frequency monitor; not consumed as functional control or data} \
        -tags {diagnostic clock_monitor} -from $source -to $target
}

# pcb_version_id is strapped on the PCB and remains constant while the FPGA is
# configured and running.  Bind each strap to its exact readback arithmetic FD.
foreach {port_name destination} {
    {pcb_version_id[0]} {saturn_top_i/PCIe/c_addsub_0/U0/xst_addsub/i_baseblox.i_baseblox_addsub/no_pipelining.the_addsub/i_lut6.i_lut6_addsub/i_q.i_simple.qreg/i_no_async_controls.output_reg[1]/D}
    {pcb_version_id[1]} {saturn_top_i/PCIe/c_addsub_0/U0/xst_addsub/i_baseblox.i_baseblox_addsub/no_pipelining.the_addsub/i_lut6.i_lut6_addsub/i_q.i_simple.qreg/i_no_async_controls.output_reg[2]/D}
    {pcb_version_id[2]} {saturn_top_i/PCIe/c_addsub_0/U0/xst_addsub/i_baseblox.i_baseblox_addsub/no_pipelining.the_addsub/i_lut6.i_lut6_addsub/i_q.i_simple.qreg/i_no_async_controls.output_reg[3]/D}
    {pcb_version_id[3]} {saturn_top_i/PCIe/c_addsub_0/U0/xst_addsub/i_baseblox.i_baseblox_addsub/no_pipelining.the_addsub/i_lut6.i_lut6_addsub/i_q.i_simple.qreg/i_no_async_controls.output_reg[4]/D}
    {pcb_version_id[3]} {saturn_top_i/PCIe/c_addsub_0/U0/xst_addsub/i_baseblox.i_baseblox_addsub/no_pipelining.the_addsub/i_lut6.i_lut6_addsub/i_q.i_simple.qreg/i_no_async_controls.output_reg[5]/D}
} {
    set source [saturn_lab::require_cdc_endpoints [get_ports -quiet $port_name] 1 \
        "$port_name static strap"]
    set target [saturn_lab::require_cdc_endpoints [get_pins -quiet $destination] 1 \
        "$port_name readback endpoint"]
    create_waiver -type CDC -id CDC-1 -user saturn-fpga \
        -description {PCB revision strap is static from before configuration through operation and is only used for board-identification readback} \
        -tags {board_contract static_strap} -from $source -to $target
}

# These SPI inputs are source-synchronous protocol data.  Their local state
# machines generate SCLK from clk122 and sample MISO on the opposite SCLK edge:
# ADC half-cycle margin is eight clk122 cycles; codec margin is six cycles.
foreach {port_name destination description} {
    ADC_MISO {saturn_top_i/PCIe/AXI_SPI_ADC_0/inst/ADCData_reg[0]/D} {ADC MISO is sampled eight clk122 cycles after the generated SCLK transition}
    CODEC_MISO {saturn_top_i/PCIe/AXIL_SPIWriter_0/inst/shiftinreg_reg[0]/D} {Codec MISO is sampled six clk122 cycles after the generated SCLK transition}
} {
    set source [saturn_lab::require_cdc_endpoints [get_ports -quiet $port_name] 1 \
        "$port_name SPI source"]
    set target [saturn_lab::require_cdc_endpoints [get_pins -quiet $destination] 1 \
        "$port_name SPI sampling endpoint"]
    create_waiver -type CDC -id CDC-1 -user saturn-fpga -description $description \
        -tags {source_synchronous spi} -from $source -to $target
}

# Active-low reset causes from clk122 and the synchronized global reset are
# ANDed for asynchronous assertion.  Each AXIS FIFO's XPM reset block then
# synchronizes release independently into its write and read clock domains.
set reset_dest_duc {saturn_top_i/FIFO_Interfaces/axis_data_fifo_DUC/inst/gen_fifo.xpm_fifo_axis_inst/gaxis_rst_sync.xpm_cdc_sync_rst_inst/syncstages_ff_reg[0]/D}
set reset_dest_codec {saturn_top_i/FIFO_Interfaces/axis_data_fifo_codecspk/inst/gen_fifo.xpm_fifo_axis_inst/gaxis_rst_sync.xpm_cdc_sync_rst_inst/syncstages_ff_reg[0]/D}
foreach {source_name destination description} [list \
    {saturn_top_i/PCIe/AXIL_ConfigReg_64_1/inst/config_reg0_reg[3]/C} $reset_dest_duc {Software DUC reset and synchronized global reset are active-low asynchronous assertion causes; XPM synchronizes release per FIFO domain} \
    {saturn_top_i/PCIe/AXIL_ConfigReg_64_1/inst/config_reg0_reg[1]/C} $reset_dest_codec {Software codec-speaker reset and synchronized global reset are active-low asynchronous assertion causes; XPM synchronizes release per FIFO domain} \
    {saturn_top_i/PCIe/Double_D_register_syncareset1/inst/Intermediate2_reg[0]/C} $reset_dest_duc {Synchronized global reset and software DUC reset are active-low asynchronous assertion causes; XPM synchronizes release per FIFO domain} \
    {saturn_top_i/PCIe/Double_D_register_syncareset1/inst/Intermediate2_reg[0]/C} $reset_dest_codec {Synchronized global reset and software codec-speaker reset are active-low asynchronous assertion causes; XPM synchronizes release per FIFO domain}] {
    set source [saturn_lab::require_cdc_endpoints [get_pins -quiet $source_name] 1 \
        {FIFO reset source clock endpoint}]
    set target [saturn_lab::require_cdc_endpoints [get_pins -quiet $destination] 1 \
        {FIFO XPM reset synchronizer endpoint}]
    create_waiver -type CDC -id CDC-12 -user saturn-fpga -description $description \
        -tags {reset async_assert sync_release xpm_fifo} -from $source -to $target
    # Once the multi-clock topology is explicitly waived, Vivado also exposes
    # CDC-10 for the deliberate AND gate that combines the active-low causes.
    create_waiver -type CDC -id CDC-10 -user saturn-fpga -description $description \
        -tags {reset async_assert sync_release xpm_fifo} -from $source -to $target
}
