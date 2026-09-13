source [file join [file dirname [file normalize [info script]]] common.tcl]

saturn_lab::vivado_guard
set project_file [file join $saturn_lab::fpga_dir saturn_project saturn_project.xpr]
saturn_lab::require_file $project_file "Saturn Vivado project"
open_project $project_file

set block_design [get_files -quiet -norecurse saturn_top.bd]
if {[llength $block_design] != 1} {
    error "Expected one saturn_top.bd; found [llength $block_design]"
}
open_bd_design $block_design

proc saturn_lab::insert_tx_enable_sync {hierarchy sink clock} {
    set saved [current_bd_instance .]
    set container [get_bd_cells -quiet $hierarchy]
    if {[llength $container] != 1} {
        error "CDC migration hierarchy not found: $hierarchy"
    }
    current_bd_instance $container

    set sync [get_bd_cells -quiet Double_D_register_TX_ENABLE]
    if {[llength $sync] == 0} {
        set sync [create_bd_cell -type module -reference Double_D_register \
            Double_D_register_TX_ENABLE]
        set_property CONFIG.DATA_WIDTH {1} $sync
    } elseif {[llength $sync] != 1} {
        error "Expected at most one TX_ENABLE synchronizer in $hierarchy"
    }

    set sink_pin [get_bd_pins -quiet $sink]
    if {[llength $sink_pin] != 1} {
        error "TX_ENABLE sink not found in $hierarchy: $sink"
    }
    set old_net [get_bd_nets -quiet -of_objects $sink_pin]
    set sync_out_net [get_bd_nets -quiet -of_objects \
        [get_bd_pins Double_D_register_TX_ENABLE/dout]]
    if {[llength $old_net] == 1 && $old_net ne $sync_out_net} {
        disconnect_bd_net $old_net $sink_pin
    }

    connect_bd_net [get_bd_pins TX_ENABLE] \
        [get_bd_pins Double_D_register_TX_ENABLE/din]
    connect_bd_net [get_bd_pins $clock] \
        [get_bd_pins Double_D_register_TX_ENABLE/aclk]
    connect_bd_net [get_bd_pins Double_D_register_TX_ENABLE/dout] $sink_pin
    current_bd_instance $saved
}

saturn_lab::insert_tx_enable_sync /PCIe xlconcat_0/In6 clk_122
saturn_lab::insert_tx_enable_sync /Transmitter IQ_Modulation_Select/TX_ENABLE clk122
current_bd_instance /
validate_bd_design
save_bd_design
close_project
puts "SATURN_LAB_CDC_MIGRATION_OK"
