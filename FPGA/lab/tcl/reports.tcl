proc saturn_lab::write_reports {output_dir synth_run impl_run} {
    file mkdir $output_dir

    report_timing_summary -delay_type min_max -max_paths 100 \
        -report_unconstrained -file [file join $output_dir timing-summary.txt]
    report_utilization -hierarchical \
        -file [file join $output_dir utilization-hierarchical.txt]
    report_drc -file [file join $output_dir drc.txt]
    report_clock_interaction \
        -file [file join $output_dir clock-interaction.txt]

    if {[catch {
        report_cdc -details -file [file join $output_dir cdc.txt]
    } message]} {
        set stream [open [file join $output_dir cdc-unavailable.txt] w]
        puts $stream $message
        close $stream
        puts "WARNING: report_cdc was unavailable: $message"
    }

    if {[catch {
        report_methodology -file [file join $output_dir methodology.txt]
    } message]} {
        puts "WARNING: report_methodology was unavailable: $message"
    }

    set stream [open [file join $output_dir run-status.txt] w]
    foreach run [list $synth_run $impl_run] {
        puts $stream "[get_property NAME $run]\t[get_property STATUS $run]\t[get_property PROGRESS $run]"
    }
    close $stream
}

proc saturn_lab::implementation_quality_gate {output_dir} {
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
    close $stream

    puts "Implementation quality: WNS=$wns ns, WHS=$whs ns, DRC errors=$drc_errors, DRC critical warnings=$drc_critical_warnings"
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
    if {[llength $failures] > 0} {
        error "Implementation quality gate failed: [join $failures {; }]"
    }
}
