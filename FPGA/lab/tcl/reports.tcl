proc saturn_lab::write_cdc_reports {output_dir} {
    variable cdc_critical_count

    # Establish the complete routed finding set before applying reviewed,
    # endpoint-bound waivers.  Keep raw, waived, reviewed, and gate inputs as
    # separate artifacts so every production decision remains auditable.
    report_cdc -details -no_waiver \
        -file [file join $output_dir cdc-raw.txt]
    source [file join $saturn_lab::tcl_dir cdc-waivers.tcl]
    report_cdc -details -show_waiver \
        -file [file join $output_dir cdc-reviewed.txt]
    report_cdc -details -waived \
        -file [file join $output_dir cdc-waived.txt]
    report_waivers -type CDC \
        -file [file join $output_dir cdc-waivers.txt]

    # Run the unwaived report last.  get_cdc_violations then addresses exactly
    # the result set used by the production gate, without relying on parsing a
    # human-readable report.
    report_cdc -details -name saturn_cdc_unwaived \
        -file [file join $output_dir cdc.txt]
    set critical [get_cdc_violations -name saturn_cdc_unwaived -quiet \
        -filter {SEVERITY == Critical}]
    set cdc_critical_count [llength $critical]
    puts "Unwaived CDC Critical count: $cdc_critical_count"
    return $cdc_critical_count
}

proc saturn_lab::write_reports {output_dir synth_run impl_run} {
    variable methodology_critical_count
    file mkdir $output_dir

    report_timing_summary -delay_type min_max -max_paths 100 \
        -report_unconstrained -file [file join $output_dir timing-summary.txt]
    report_utilization -hierarchical \
        -file [file join $output_dir utilization-hierarchical.txt]
    report_drc -file [file join $output_dir drc.txt]
    report_clock_interaction \
        -file [file join $output_dir clock-interaction.txt]

    write_cdc_reports $output_dir

    set methodology_critical_count -1
    if {[catch {
        report_methodology -file [file join $output_dir methodology.txt]
    } message]} {
        puts "WARNING: report_methodology was unavailable: $message"
    } else {
        set methodology_critical_count 0
        foreach violation [get_methodology_violations -quiet] {
            if {[get_property SEVERITY $violation] eq "Critical Warning"} {
                incr methodology_critical_count
            }
        }
        puts "Methodology Critical Warning count: $methodology_critical_count"
    }

    set stream [open [file join $output_dir run-status.txt] w]
    foreach run [list $synth_run $impl_run] {
        puts $stream "[get_property NAME $run]\t[get_property STATUS $run]\t[get_property PROGRESS $run]"
    }
    close $stream
}

proc saturn_lab::implementation_quality_gate {output_dir} {
    variable cdc_critical_count
    variable methodology_critical_count
    set setup_paths [get_timing_paths -quiet -delay_type max -max_paths 1]
    set hold_paths [get_timing_paths -quiet -delay_type min -max_paths 1]
    if {[llength $setup_paths] == 0 || [llength $hold_paths] == 0} {
        error "Timing quality gate could not find both setup and hold paths"
    }

    set wns [get_property SLACK [lindex $setup_paths 0]]
    set whs [get_property SLACK [lindex $hold_paths 0]]
    set drc_errors 0
    set drc_critical_warnings 0
    foreach violation [get_drc_violations -quiet] {
        set severity [get_property SEVERITY $violation]
        if {$severity eq "Error"} {
            incr drc_errors
        } elseif {$severity eq "Critical Warning"} {
            incr drc_critical_warnings
        }
    }

    set gate_path [file join $output_dir quality-gate.txt]
    set stream [open $gate_path w]
    puts $stream "WNS_NS\t$wns"
    puts $stream "WHS_NS\t$whs"
    puts $stream "DRC_ERRORS\t$drc_errors"
    puts $stream "DRC_CRITICAL_WARNINGS\t$drc_critical_warnings"
    puts $stream "CDC_CRITICAL_UNWAIVED\t$cdc_critical_count"
    puts $stream "METHODOLOGY_CRITICAL_WARNINGS\t$methodology_critical_count"
    close $stream

    puts "Implementation quality: WNS=$wns ns, WHS=$whs ns, DRC errors=$drc_errors, DRC critical warnings=$drc_critical_warnings, unwaived CDC critical=$cdc_critical_count, methodology critical warnings=$methodology_critical_count"
    set failures {}
    if {$wns < 0.0} {
        lappend failures "negative setup slack ($wns ns)"
    }
    if {$whs < 0.0} {
        lappend failures "negative hold slack ($whs ns)"
    }
    if {$drc_errors > 0} {
        lappend failures "$drc_errors DRC error(s)"
    }
    if {$drc_critical_warnings > 0} {
        lappend failures "$drc_critical_warnings DRC Critical Warning(s)"
    }
    if {$cdc_critical_count > 0} {
        lappend failures "$cdc_critical_count unwaived CDC Critical finding(s)"
    }
    if {$methodology_critical_count < 0} {
        lappend failures "methodology report unavailable"
    } elseif {$methodology_critical_count > 0} {
        lappend failures "$methodology_critical_count methodology Critical Warning(s)"
    }
    if {[llength $failures] > 0} {
        error "Implementation quality gate failed: [join $failures {; }]"
    }
}

proc saturn_lab::telemetry_netlist_gate {output_dir} {
    set checks [list \
        fifo_event_counter {.*FIFO_Monitor_0/inst/fifo1_events_reg.*} \
        snapshot_sequence {.*FIFO_Monitor_0/inst/snapshot_sequence_reg.*} \
        extended_read_address {.*FIFO_Monitor_0/inst/raddrreg_reg\[6\].*} \
        adc_episode_counter {.*AXI_FIFO_overflow_re_1/inst/ADC1episodecountreg_reg.*} \
        adc_episode_duration {.*AXI_FIFO_overflow_re_1/inst/ADC1totalhighreg_reg.*} \
        adc_extended_read_address {.*AXI_FIFO_overflow_re_1/inst/raddrreg_reg\[6\].*}]
    set report_path [file join $output_dir telemetry-netlist-gate.txt]
    set stream [open $report_path w]
    set failures {}
    foreach {label pattern} $checks {
        set matches [get_cells -hierarchical -regexp -quiet $pattern]
        set count [llength $matches]
        puts $stream "$label\t$count\t$pattern"
        puts "Telemetry netlist: $label cells=$count"
        if {$count == 0} {
            lappend failures $label
        }
    }
    close $stream
    if {[llength $failures] > 0} {
        error "Routed telemetry netlist is missing required V29/V30 state: [join $failures {, }]"
    }
    puts "SATURN_LAB_TELEMETRY_NETLIST_OK report=$report_path"
}
