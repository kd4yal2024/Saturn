source [file join [file dirname [file normalize [info script]]] common.tcl]
source [file join [file dirname [file normalize [info script]]] reports.tcl]

set vivado_version [saturn_lab::vivado_guard]
set project_file [file join $saturn_lab::fpga_dir saturn_project saturn_project.xpr]
saturn_lab::require_file $project_file "Saturn Vivado project"

set synth_name [saturn_lab::env_or SATURN_SYNTH_RUN synth_2_copy_1]
set impl_name [saturn_lab::env_or SATURN_IMPL_RUN impl_1_copy_1]
set output_dir [file join $saturn_lab::results_dir vivado]
file mkdir $output_dir

puts "Opening $project_file"
open_project $project_file
saturn_lab::ensure_managed_wrapper
set synth_run [saturn_lab::require_run $synth_name]
set impl_run [saturn_lab::require_run $impl_name]
update_compile_order -fileset sources_1

set reuse_synth [saturn_lab::env_or SATURN_REUSE_SYNTH 0]
if {$reuse_synth ni {0 1}} {
    error "SATURN_REUSE_SYNTH must be 0 or 1"
}

# Allow a timing-closure retry to select stronger implementation directives
# while keeping the production defaults in the project. Vivado validates each
# value when it is assigned, so misspelled or unsupported directives fail fast.
foreach {environment property} {
    SATURN_PLACE_DIRECTIVE STEPS.PLACE_DESIGN.ARGS.DIRECTIVE
    SATURN_PHYSOPT_DIRECTIVE STEPS.PHYS_OPT_DESIGN.ARGS.DIRECTIVE
    SATURN_ROUTE_DIRECTIVE STEPS.ROUTE_DESIGN.ARGS.DIRECTIVE
    SATURN_POST_ROUTE_PHYSOPT_DIRECTIVE STEPS.POST_ROUTE_PHYS_OPT_DESIGN.ARGS.DIRECTIVE
} {
    set directive [saturn_lab::env_or $environment ""]
    if {$directive ne ""} {
        puts "Setting $property=$directive on $impl_name"
        set_property $property $directive $impl_run
    }
}

if {[saturn_lab::env_or SATURN_SKIP_RESET 0] ne "1"} {
    reset_run $impl_run
    if {!$reuse_synth} {
        reset_run $synth_run
    }
}

if {!$reuse_synth} {
    launch_runs $synth_run -jobs [saturn_lab::jobs]
    wait_on_run $synth_run
} else {
    puts "Reusing completed synthesis run $synth_name"
}
saturn_lab::assert_run_complete $synth_run

launch_runs $impl_run -to_step write_bitstream -jobs [saturn_lab::jobs]
wait_on_run $impl_run
saturn_lab::assert_run_complete $impl_run

open_run $impl_run
saturn_lab::write_reports $output_dir $synth_run $impl_run
saturn_lab::implementation_quality_gate $output_dir

set run_dir [get_property DIRECTORY $impl_run]
set preferred [file join $run_dir saturn_top_wrapper.bit]
if {[file isfile $preferred]} {
    set bitstream $preferred
} else {
    set candidates [glob -nocomplain -directory $run_dir *.bit]
    if {[llength $candidates] != 1} {
        error "Expected one bitstream in $run_dir; found [llength $candidates]"
    }
    set bitstream [lindex $candidates 0]
}

set short_sha [string range [saturn_lab::git_value rev-parse HEAD] 0 7]
set artifact [file join $output_dir "saturn-${short_sha}.bit"]
file copy -force $bitstream $artifact
saturn_lab::write_manifest [file join $output_dir manifest.json] $artifact \
    $vivado_version $synth_name $impl_name

puts "SATURN_LAB_BUILD_OK artifact=$artifact"
close_project
