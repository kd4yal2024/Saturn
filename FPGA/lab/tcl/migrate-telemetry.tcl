# Persist the V29 FIFO almost-full telemetry wiring into the checked-in
# saturn_top block design.  The generated project-recreation script carries the
# same edits, but existing projects need this one-time migration.
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

# The monitor and both overflow readers are module-reference IP.  Vivado does
# not automatically invalidate their OOC checkpoints when the referenced RTL
# changes, so refresh their generated wrappers before regenerating the BD.
set telemetry_module_refs [get_ips -quiet [list \
    saturn_top_FIFO_Monitor_0_0 \
    saturn_top_AXI_FIFO_overflow_re_0_1 \
    saturn_top_AXI_FIFO_overflow_re_0_2]]
if {[llength $telemetry_module_refs] != 3} {
    error "Expected three telemetry module references; found [llength $telemetry_module_refs]"
}
update_module_reference $telemetry_module_refs

set fifo_names {axis_data_fifo_DDC0 axis_data_fifo_DUC axis_data_fifo_codecmic axis_data_fifo_codecspk}
set monitor [get_bd_cells -quiet /FIFO_Interfaces/FIFO_Monitor_0]
if {[llength $monitor] != 1} {
    error "FIFO_Monitor_0 not found under /FIFO_Interfaces"
}

foreach fifo_name $fifo_names {
    set fifo [get_bd_cells -quiet "/FIFO_Interfaces/$fifo_name"]
    if {[llength $fifo] != 1} {
        error "FIFO not found: $fifo_name"
    }
    set_property CONFIG.HAS_AFULL {1} $fifo
    set source [get_bd_pins -quiet "/FIFO_Interfaces/$fifo_name/almost_full"]
    set sink [get_bd_pins -quiet "/FIFO_Interfaces/FIFO_Monitor_0/fifo[expr {[lsearch -exact $fifo_names $fifo_name] + 1}]_overflow"]
    if {[llength $source] != 1 || [llength $sink] != 1} {
        error "Telemetry pin missing for $fifo_name"
    }
    foreach old_net [get_bd_nets -quiet -of_objects $sink] {
        disconnect_bd_net $old_net $sink
    }
    connect_bd_net $source $sink
}

validate_bd_design
save_bd_design
generate_target all $block_design
export_ip_user_files -of_objects $block_design -no_script -sync -force -quiet
close_project
puts "SATURN_LAB_TELEMETRY_MIGRATION_OK"
