namespace eval saturn_lab {
    variable tcl_dir [file dirname [file normalize [info script]]]
    variable lab_dir [file dirname $tcl_dir]
    variable fpga_dir [file dirname $lab_dir]
    variable repo_dir [file dirname $fpga_dir]
    variable results_dir [file join $lab_dir results]

    proc env_or {name fallback} {
        if {[info exists ::env($name)] && $::env($name) ne ""} {
            return $::env($name)
        }
        return $fallback
    }

    proc require_file {path description} {
        if {![file isfile $path]} {
            error "$description not found: $path"
        }
    }

    proc vivado_guard {} {
        set actual [version -short]
        set expected "2023.1"
        if {![string match "${expected}*" $actual]} {
            if {[env_or SATURN_ALLOW_UNSUPPORTED_VIVADO 0] ne "1"} {
                error "Saturn requires Vivado $expected; found $actual. Set SATURN_ALLOW_UNSUPPORTED_VIVADO=1 only for an intentional experiment."
            }
            puts "WARNING: continuing with unsupported Vivado $actual"
        }
        return $actual
    }

    proc ensure_managed_wrapper {} {
        set block_design [get_files -quiet -norecurse saturn_top.bd]
        if {[llength $block_design] != 1} {
            error "Expected one saturn_top.bd; found [llength $block_design]"
        }

        set wrapper [get_files -quiet *saturn_top_wrapper.v]
        set needs_generation [expr {[llength $wrapper] != 1}]
        if {!$needs_generation} {
            set needs_generation [expr {![file isfile [lindex $wrapper 0]]}]
        }
        if {$needs_generation} {
            puts "Generating the managed saturn_top HDL wrapper"
            set wrapper_path [make_wrapper -fileset sources_1 -files $block_design -top]
            if {[llength [get_files -quiet $wrapper_path]] == 0} {
                add_files -norecurse -fileset sources_1 $wrapper_path
            }
        }

        set_property top saturn_top_wrapper [get_filesets sources_1]
    }

    proc jobs {} {
        set value [env_or SATURN_VIVADO_JOBS 8]
        if {![string is integer -strict $value] || $value < 1} {
            error "SATURN_VIVADO_JOBS must be a positive integer"
        }
        return $value
    }

    proc git_value {args} {
        variable repo_dir
        if {[catch {exec git -C $repo_dir {*}$args} output]} {
            return "unknown"
        }
        return [string trim $output]
    }

    proc git_dirty {} {
        variable repo_dir
        if {[info exists ::env(SATURN_GIT_DIRTY)] && $::env(SATURN_GIT_DIRTY) ne ""} {
            return $::env(SATURN_GIT_DIRTY)
        }
        if {[catch {exec git -C $repo_dir diff --quiet --ignore-submodules --}]} {
            return true
        }
        if {[catch {exec git -C $repo_dir diff --cached --quiet --ignore-submodules --}]} {
            return true
        }
        if {[git_value status --porcelain] ne ""} {
            return true
        }
        return false
    }

    proc sha256 {path} {
        if {$path eq "" || ![file isfile $path]} {
            return ""
        }
        if {![catch {exec sha256sum $path} output]} {
            return [lindex [split [string trim $output]] 0]
        }
        if {![catch {exec certutil -hashfile $path SHA256} output]} {
            foreach line [split $output "\n"] {
                set candidate [string map {" " "" "\r" ""} $line]
                if {[regexp {^[0-9A-Fa-f]{64}$} $candidate]} {
                    return [string tolower $candidate]
                }
            }
        }
        return "unavailable"
    }

    proc json_escape {value} {
        return [string map [list "\\" "\\\\" "\"" "\\\"" "\n" "\\n" "\r" "\\r" "\t" "\\t"] $value]
    }

    proc write_manifest {path artifact vivado_version synth_run impl_run} {
        set sha [git_value rev-parse HEAD]
        set branch [git_value branch --show-current]
        set timestamp [clock format [clock seconds] -gmt true -format {%Y-%m-%dT%H:%M:%SZ}]
        set artifact_name ""
        if {$artifact ne ""} {
            set artifact_name [file tail $artifact]
        }
        set stream [open $path w]
        puts $stream "{"
        puts $stream "  \"schema\": 1,"
        puts $stream "  \"created_utc\": \"[json_escape $timestamp]\","
        puts $stream "  \"git_branch\": \"[json_escape $branch]\","
        puts $stream "  \"git_sha\": \"[json_escape $sha]\","
        puts $stream "  \"git_dirty\": [git_dirty],"
        puts $stream "  \"vivado\": \"[json_escape $vivado_version]\","
        puts $stream "  \"synthesis_run\": \"[json_escape $synth_run]\","
        puts $stream "  \"implementation_run\": \"[json_escape $impl_run]\","
        puts $stream "  \"artifact\": \"[json_escape $artifact_name]\","
        puts $stream "  \"sha256\": \"[json_escape [sha256 $artifact]]\""
        puts $stream "}"
        close $stream
    }

    proc require_run {name} {
        set run [get_runs -quiet $name]
        if {[llength $run] != 1} {
            error "Vivado run '$name' does not exist in [current_project]"
        }
        return $run
    }

    proc assert_run_complete {run} {
        set status [get_property STATUS $run]
        set progress [get_property PROGRESS $run]
        puts "Run [get_property NAME $run]: status='$status', progress='$progress'"
        if {$progress ne "100%" || ![string match "*Complete*" $status]} {
            error "Run [get_property NAME $run] did not complete successfully"
        }
    }
}
