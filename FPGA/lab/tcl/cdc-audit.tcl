source [file join [file dirname [file normalize [info script]]] common.tcl]
source [file join [file dirname [file normalize [info script]]] reports.tcl]

saturn_lab::vivado_guard
set project_file [file join $saturn_lab::fpga_dir saturn_project saturn_project.xpr]
saturn_lab::require_file $project_file "Saturn Vivado project"
set impl_name [saturn_lab::env_or SATURN_IMPL_RUN impl_1_copy_1]
set output_dir [file join $saturn_lab::results_dir vivado]
file mkdir $output_dir

open_project $project_file
set impl_run [saturn_lab::require_run $impl_name]
saturn_lab::assert_run_complete $impl_run
open_run $impl_run

set critical_count [saturn_lab::write_cdc_reports $output_dir]
if {$critical_count != 0} {
    error "CDC quality gate failed with $critical_count unwaived Critical finding(s)"
}

close_project
puts "SATURN_LAB_CDC_OK results=$output_dir"
