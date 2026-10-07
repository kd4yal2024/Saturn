# Generate the RXC1 candidate from the reviewed revision-4 routed checkpoint.
# Usage: vivado -mode batch -source bitgen-rxc1.tcl -tclargs CHECKPOINT BIT BIN
# This script does not synthesize, route, program, or touch the installed G2.
if {[llength $argv] != 3} {
    error "expected CHECKPOINT BIT BIN"
}
lassign $argv checkpoint bitfile binfile
if {![file isfile $checkpoint]} {
    error "missing reviewed checkpoint: $checkpoint"
}
if {[file exists $bitfile] || [file exists $binfile]} {
    error "refusing to overwrite candidate artifacts"
}
if {[file dirname $bitfile] ne [file dirname $binfile]} {
    error "BIT and BIN must share one output directory"
}
open_checkpoint $checkpoint
set previous [get_property BITSTREAM.CONFIG.USR_ACCESS [current_design]]
if {![string equal -nocase $previous 0x53460003]} {
    error "unexpected parent USR_ACCESS: $previous"
}
set part [get_property PART [current_design]]
if {![string equal -nocase $part xc7a200tfbg676-2]} {
    error "unexpected FPGA part: $part"
}
set_property BITSTREAM.CONFIG.USR_ACCESS 0x53460004 [current_design]
if {![string equal -nocase [get_property BITSTREAM.CONFIG.USR_ACCESS [current_design]] 0x53460004]} {
    error "candidate USR_ACCESS did not bind"
}
write_bitstream $bitfile
write_cfgmem -format bin -size 32 -interface SPIx1 -loadbit [list up 0x00000000 $bitfile] $binfile
set capacity [expr {0x01300000 - 0x00980000}]
if {![file isfile $bitfile] || ![file isfile $binfile] ||
    [file size $bitfile] == 0 || [file size $binfile] == 0 ||
    [file size $binfile] > $capacity} {
    error "invalid candidate BIT/BIN size"
}
puts "RXC1_CANDIDATE_BITGEN_OK parent_usr_access=$previous candidate_usr_access=0x53460004 part=$part bit_bytes=[file size $bitfile] bin_bytes=[file size $binfile] primary_capacity=$capacity"
close_design
