source [file join [file dirname [file normalize [info script]]] common.tcl]

set vivado_version [saturn_lab::vivado_guard]
set project_file [file join $saturn_lab::fpga_dir saturn_project saturn_project.xpr]
saturn_lab::require_file $project_file "Saturn Vivado project"

set output_dir [file join $saturn_lab::results_dir vivado]
file mkdir $output_dir
set report_file [file join $output_dir validation.txt]

puts "Opening $project_file"
open_project $project_file
saturn_lab::ensure_managed_wrapper
# Validation is intentionally metadata-only. Vivado 2023.1 can hang while
# refreshing compile order in a project migrated from an older release. The
# build and simulation flows perform this refresh when it is required.
if {[saturn_lab::env_or SATURN_VALIDATE_UPDATE_COMPILE_ORDER 0] eq "1"} {
    update_compile_order -fileset sources_1
}

set expected_part xc7a200tfbg676-2
set actual_part [get_property PART [current_project]]
if {$actual_part ne $expected_part} {
    error "Expected part $expected_part; found $actual_part"
}
set actual_top [get_property TOP [get_filesets sources_1]]
if {$actual_top ne "saturn_top_wrapper"} {
    error "Expected top saturn_top_wrapper; found $actual_top"
}
set locked_ip [get_ips -quiet -filter {IS_LOCKED == 1}]
if {[llength $locked_ip] != 0} {
    error "Locked IP detected: $locked_ip"
}

set stream [open $report_file w]
puts $stream "Saturn Vivado project validation"
puts $stream "Vivado: $vivado_version"
puts $stream "Project: [get_property NAME [current_project]]"
puts $stream "Part: $actual_part"
puts $stream "Top: $actual_top"
puts $stream "Runs:"
foreach run [lsort [get_runs]] {
    puts $stream "  [get_property NAME $run] | [get_property FLOW $run] | [get_property STATUS $run]"
}
puts $stream "IP count: [llength [get_ips -quiet]]"
puts $stream "Locked IP count: [llength $locked_ip]"
close $stream

puts "SATURN_LAB_VALIDATE_OK report=$report_file"
close_project
