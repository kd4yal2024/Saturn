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

# The host uses this constant as the public firmware revision.  Keep it tied to
# the telemetry ABI implemented by this candidate instead of silently emitting
# a new image that still advertises the legacy V27 contract.
set expected_firmware_version 29
set firmware_version_ip [get_ips -quiet saturn_top_xlconstant_5_0]
if {[llength $firmware_version_ip] != 1} {
    error "Expected one firmware-version IP saturn_top_xlconstant_5_0; found [llength $firmware_version_ip]"
}
set actual_firmware_version [get_property CONFIG.CONST_VAL $firmware_version_ip]
if {$actual_firmware_version != $expected_firmware_version} {
    error "Firmware identity mismatch: expected $expected_firmware_version, found $actual_firmware_version"
}
puts "Firmware identity: version=$actual_firmware_version, USR_ACCESS date=09122026"

set synth_run [saturn_lab::require_run $synth_name]
set impl_run [saturn_lab::require_run $impl_name]

set reuse_synth [saturn_lab::env_or SATURN_REUSE_SYNTH 0]
if {$reuse_synth ni {0 1}} {
    error "SATURN_REUSE_SYNTH must be 0 or 1"
}

set synth_identity "firmware_version=$expected_firmware_version\nusr_access=09122026"
set synth_identity_stamp [file join $output_dir synth-identity.txt]
if {$reuse_synth} {
    if {![file isfile $synth_identity_stamp]} {
        error "Synthesis reuse is unsafe: $synth_identity_stamp is missing. Run once without SATURN_REUSE_SYNTH=1."
    }
    set stamp_stream [open $synth_identity_stamp r]
    set stamped_identity [string trim [read $stamp_stream]]
    close $stamp_stream
    if {$stamped_identity ne $synth_identity} {
        error "Synthesis reuse is unsafe: checkpoint identity is '$stamped_identity', expected '$synth_identity'. Run once without SATURN_REUSE_SYNTH=1."
    }
}

# These module-reference IPs contain locally maintained RTL.  Vivado can leave
# their OOC synthesis runs marked complete after the source changes, causing a
# top-level build to silently stitch an old DCP into the new design.  Refresh
# the wrappers and invalidate those checkpoints for every non-reuse build.
set telemetry_module_ref_names [list \
    saturn_top_FIFO_Monitor_0_0 \
    saturn_top_AXI_FIFO_overflow_re_0_1 \
    saturn_top_AXI_FIFO_overflow_re_0_2]
if {!$reuse_synth} {
    # The generated xlconstant wrapper is a synthesis input and may still hold
    # the preceding release value even after the checked-in BD/XCI is edited.
    # Nested IP products must be regenerated through their parent block design.
    puts "Regenerating firmware-version constant"
    set top_block_design [get_files -quiet -norecurse saturn_top.bd]
    if {[llength $top_block_design] != 1} {
        error "Expected one saturn_top.bd; found [llength $top_block_design]"
    }
    if {[get_property IS_LOCKED $firmware_version_ip]} {
        puts "Refreshing locked firmware-version IP"
        upgrade_ip $firmware_version_ip
    }
    reset_target all $top_block_design
    generate_target all $top_block_design

    set telemetry_module_refs [get_ips -quiet $telemetry_module_ref_names]
    if {[llength $telemetry_module_refs] != [llength $telemetry_module_ref_names]} {
        error "Expected [llength $telemetry_module_ref_names] telemetry module references; found [llength $telemetry_module_refs]"
    }
    puts "Refreshing telemetry module references"
    update_module_reference $telemetry_module_refs
}
update_compile_order -fileset sources_1

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
        foreach module_ref_name $telemetry_module_ref_names {
            set module_runs [get_runs -quiet "${module_ref_name}_synth_1"]
            if {[llength $module_runs] == 1} {
                puts "Invalidating stale module-reference checkpoint [get_property NAME $module_runs]"
                reset_run $module_runs
            } elseif {[llength $module_runs] == 0} {
                # update_module_reference removes an existing OOC run when it
                # refreshes the module. launch_runs will recreate it as needed.
                puts "Module-reference checkpoint for $module_ref_name is already invalidated"
            } else {
                error "Expected at most one synthesis run for $module_ref_name; found [llength $module_runs]"
            }
        }
    }
}

if {!$reuse_synth} {
    launch_runs $synth_run -jobs [saturn_lab::jobs]
    wait_on_run $synth_run
} else {
    puts "Reusing completed synthesis run $synth_name"
}
saturn_lab::assert_run_complete $synth_run
if {!$reuse_synth} {
    set stamp_stream [open $synth_identity_stamp w]
    puts -nonewline $stamp_stream $synth_identity
    close $stamp_stream
}

launch_runs $impl_run -to_step write_bitstream -jobs [saturn_lab::jobs]
wait_on_run $impl_run
saturn_lab::assert_run_complete $impl_run

open_run $impl_run
saturn_lab::write_reports $output_dir $synth_run $impl_run
saturn_lab::implementation_quality_gate $output_dir
saturn_lab::telemetry_netlist_gate $output_dir

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
set artifact [file join $output_dir "saturn-v${expected_firmware_version}-${short_sha}.bit"]
file copy -force $bitstream $artifact
saturn_lab::write_manifest [file join $output_dir manifest.json] $artifact \
    $vivado_version $synth_name $impl_name $expected_firmware_version

puts "SATURN_LAB_BUILD_OK artifact=$artifact"
close_project
