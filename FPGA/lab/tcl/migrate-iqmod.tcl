source [file join [file dirname [file normalize [info script]]] sim-common.tcl]

saturn_lab::vivado_guard
set project_file [file join $saturn_lab::fpga_dir IP CODEC_IQMOD_IP CODEC_IQMOD_IP.xpr]
saturn_lab::require_file $project_file {IQMod Vivado project}
set output_dir [file join $saturn_lab::results_dir vivado iqmod-ip-migration]
file mkdir $output_dir

open_project $project_file
set locked_before [get_ips -quiet -filter {IS_LOCKED == 1}]
puts "IQMod locked IP count before migration: [llength $locked_before]"
if {[llength $locked_before] > 0} {
    upgrade_ip $locked_before
}

set locked_after [get_ips -quiet -filter {IS_LOCKED == 1}]
if {[llength $locked_after] > 0} {
    error "IQMod IP migration left locked instances: $locked_after"
}

# Refresh the source-BD module references whose checked-in RTL changed during
# the production hardening. Nested container copies are regenerated below.
set changed_module_refs [get_ips -quiet [list \
    IQ_Modulation_Select_axis_mux_4_0_0 \
    IQ_Modulation_Select_cw_key_ramp_0_0]]
if {[llength $changed_module_refs] != 2} {
    error "Expected two changed IQMod module references; found [llength $changed_module_refs]"
}
update_module_reference $changed_module_refs

# Regenerate every product after the catalog revision update. This also
# refreshes nested block-design containers and the exported simulation tree.
saturn_lab::generate_simulation_products
foreach block_design [get_files -quiet -norecurse *.bd] {
    open_bd_design $block_design
    validate_bd_design
    save_bd_design
}
report_ip_status -file [file join $output_dir ip-status.txt]
close_project

puts "SATURN_LAB_IQMOD_IP_MIGRATION_OK results=$output_dir"
