source [file join [file dirname [file normalize [info script]]] common.tcl]

proc saturn_lab::find_named_files {root pattern} {
    set matches {}
    if {![file isdirectory $root]} {
        return $matches
    }
    foreach entry [glob -nocomplain -directory $root *] {
        if {[file isdirectory $entry]} {
            set matches [concat $matches [find_named_files $entry $pattern]]
        } elseif {[string match $pattern [file tail $entry]]} {
            lappend matches $entry
        }
    }
    return $matches
}

proc saturn_lab::register_generated_simulation_sources {} {
    set project_dir [get_property DIRECTORY [current_project]]
    set project_name [get_property NAME [current_project]]
    set generated_dir [file join $project_dir "${project_name}.gen"]
    set vip_packages [find_named_files $generated_dir *_axi_vip_*_pkg.sv]
    foreach vip_package $vip_packages {
        if {[llength [get_files -quiet $vip_package]] == 0} {
            puts "Registering generated AXI VIP package [file tail $vip_package]"
            add_files -norecurse -fileset sim_1 $vip_package
        }
    }
}

proc saturn_lab::sync_nested_bd_simulation_sources {} {
    # Vivado 2023.1 can regenerate the outer BD wrapper under .gen while
    # leaving the older hierarchical-child wrapper in .ip_user_files. The
    # latter is what the checked-in simulation fileset references, so a
    # 2021.2 child (with HAS_BURST/CACHE/LOCK omitted) can be elaborated
    # against a 2023.1 parent that connects those ports.
    set project_dir [get_property DIRECTORY [current_project]]
    set project_name [get_property NAME [current_project]]
    set generated_bd_root [file join $project_dir "${project_name}.gen" sources_1 bd]
    set user_bd_root [file join $project_dir "${project_name}.ip_user_files" bd]

    foreach sim_dir [glob -nocomplain -types d -directory $generated_bd_root */bd/*/sim] {
        set child_name [file tail [file dirname $sim_dir]]
        set generated_top [file join $sim_dir "${child_name}.v"]
        set user_sim_dir [file join $user_bd_root $child_name sim]
        set user_top [file join $user_sim_dir "${child_name}.v"]
        if {![file isfile $generated_top] || ![file isfile $user_top]} {
            continue
        }
        puts "Synchronizing nested BD simulation wrapper $child_name"
        file mkdir $user_sim_dir
        file copy -force $generated_top $user_top
    }
}

proc saturn_lab::generate_simulation_products {} {
    set locked_ips [get_ips -quiet -filter {IS_LOCKED == 1}]
    if {[llength $locked_ips] > 0} {
        puts "Upgrading [llength $locked_ips] locked IP instances in memory for Vivado [version -short] simulation"
        upgrade_ip $locked_ips
        set still_locked [get_ips -quiet -filter {IS_LOCKED == 1}]
        if {[llength $still_locked] > 0} {
            error "IP instances remain locked after upgrade: $still_locked"
        }
    }

    set block_designs [get_files -quiet -norecurse *.bd]
    foreach block_design $block_designs {
        puts "Generating simulation products for [file tail $block_design]"
        generate_target all $block_design
    }

    # Standalone XCI files (for example, the DDC test signal generator) are not
    # necessarily dependencies of a block design, so generate them explicitly.
    set ip_files [get_files -quiet -norecurse *.xci]
    foreach ip_file $ip_files {
        puts "Generating simulation products for [file tail $ip_file]"
        generate_target all $ip_file
    }

    if {[llength $block_designs] > 0 || [llength $ip_files] > 0} {
        export_ip_user_files \
            -of_objects [concat $block_designs $ip_files] \
            -no_script -sync -force -quiet
    }
    sync_nested_bd_simulation_sources
    register_generated_simulation_sources
}

proc saturn_lab::configure_wave_capture {} {
    set wave_configs [get_files -quiet *.wcfg]
    if {[env_or SATURN_SIM_WAVES 0] eq "1"} {
        puts "Wave capture enabled by SATURN_SIM_WAVES=1"
        return
    }

    # The checked-in GUI wave configurations recursively trace large Xilinx
    # DSP blocks and make unattended numerical regressions unnecessarily slow.
    foreach wave_config $wave_configs {
        remove_files -fileset sim_1 $wave_config
    }
    if {[llength $wave_configs] > 0} {
        puts "Wave configuration disabled for batch regression; set SATURN_SIM_WAVES=1 to enable it"
    }
}

proc saturn_lab::run_simulation {project_file top label runtime {simulator_options ""} {capture_file ""} {expected_lines 0}} {
    vivado_guard
    require_file $project_file "$label simulation project"
    set output_dir [file join $saturn_lab::results_dir simulation $label]
    file mkdir $output_dir

    puts "Opening $project_file for $label simulation"
    open_project $project_file
    set simset [get_filesets -quiet sim_1]
    if {[llength $simset] != 1} {
        error "Simulation fileset sim_1 is missing from $project_file"
    }
    set_property top $top $simset
    set_property xsim.simulate.runtime $runtime $simset
    if {$simulator_options ne ""} {
        set_property -dict [list xsim.simulate.xsim.more_options $simulator_options] $simset
    }
    generate_simulation_products
    configure_wave_capture
    update_compile_order -fileset $simset
    launch_simulation -simset $simset -mode behavioral
    close_sim

    set project_dir [file dirname $project_file]
    set project_name [file rootname [file tail $project_file]]
    set sim_dir [file join $project_dir "${project_name}.sim" sim_1 behav xsim]
    set simulator_log [file join $sim_dir simulate.log]
    if {![file isfile $simulator_log]} {
        error "$label simulator log was not created: $simulator_log"
    }
    set log_stream [open $simulator_log r]
    set log_text [read $log_stream]
    close $log_stream
    if {[regexp -nocase {simulation engine not responding|terminated in an unexpected manner|fatal:|(^|\n)error:} $log_text]} {
        error "$label simulator log reports a fatal error: $simulator_log"
    }
    if {![string match {*$finish called*} $log_text]} {
        error "$label testbench did not reach \$finish: $simulator_log"
    }
    if {$capture_file ne ""} {
        set capture_path [file join $sim_dir $capture_file]
        if {![file isfile $capture_path]} {
            error "$label capture was not created: $capture_path"
        }
        set capture_stream [open $capture_path r]
        set actual_lines 0
        while {[gets $capture_stream unused] >= 0} {
            incr actual_lines
        }
        close $capture_stream
        if {$actual_lines != $expected_lines} {
            error "$label capture contains $actual_lines lines; expected $expected_lines"
        }
    }
    foreach pattern [list *.txt *.csv *.wdb *.log] {
        foreach artifact [glob -nocomplain -directory $sim_dir $pattern] {
            file copy -force $artifact [file join $output_dir [file tail $artifact]]
        }
    }

    set stream [open [file join $output_dir simulation.txt] w]
    puts $stream "label=$label"
    puts $stream "top=$top"
    puts $stream "runtime=$runtime"
    puts $stream "vivado=[version -short]"
    puts $stream "git_sha=[git_value rev-parse HEAD]"
    close $stream

    close_project
    puts "SATURN_LAB_SIM_OK label=$label results=$output_dir"
}
