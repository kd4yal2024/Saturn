set here [file dirname [file normalize [info script]]]
source [file join $here sim-ddc.tcl]
source [file join $here sim-duc.tcl]
source [file join $here sim-iqmod.tcl]
puts "SATURN_LAB_ALL_SIMS_OK"
