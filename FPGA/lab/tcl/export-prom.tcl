# Generate the Saturn 32-Mbit SPIx1 multiboot configuration image.
#
# This is the scripted equivalent of the checked-in procedure in
# FPGA/multiboot_address_table/readme.md.  Run it from a Vivado 2023.1 Tcl
# console (or with vivado -mode batch -source export-prom.tcl).  Inputs may be
# overridden with environment variables so a lab bitstream can be exported
# without changing this file.

source [file join [file dirname [file normalize [info script]]] common.tcl]
saturn_lab::vivado_guard

set repo_dir $saturn_lab::repo_dir
set golden_bit [saturn_lab::env_or SATURN_GOLDEN_BIT \
    [file join $repo_dir FPGA multiboot_address_table saturn_top_wrapper_golden.bit]]
set current_sha [string range [saturn_lab::git_value rev-parse HEAD] 0 7]
set primary_bit [saturn_lab::env_or SATURN_PRIMARY_BIT \
    [file join $repo_dir FPGA lab results vivado "saturn-v30-${current_sha}.bit"]]
set primary_bin [saturn_lab::env_or SATURN_PRIMARY_BIN \
    [file join $repo_dir FPGA lab results vivado "saturn-primary-v30-${current_sha}.bin"]]
set timer1 [saturn_lab::env_or SATURN_TIMER1 \
    [file join $repo_dir FPGA multiboot_address_table timer1.bin]]
set timer2 [saturn_lab::env_or SATURN_TIMER2 \
    [file join $repo_dir FPGA multiboot_address_table timer2.bin]]
set output_bin [saturn_lab::env_or SATURN_PROM_OUTPUT \
    [file join $repo_dir FPGA lab results vivado saturn-lab.bin]]

# load-FPGA programs its input at the physical primary address.  Therefore it
# must receive a slot-relative BIN generated at logical address zero, never the
# complete multiboot image below.  The timer2 barrier is the exclusive upper
# bound of the primary slot.
set primary_flash_base 0x00980000
set primary_flash_limit 0x01300000

foreach input [list $golden_bit $primary_bit $timer1 $timer2] {
    saturn_lab::require_file $input "configuration input"
}
file mkdir [file dirname $output_bin]

puts "Generating primary-slot BIN for load-FPGA: $primary_bin"
write_cfgmem -format bin -size 32 -interface SPIx1 \
    -loadbit [list up 0x00000000 $primary_bit] \
    $primary_bin -force
set primary_prm [file rootname $primary_bin].prm
saturn_lab::require_file $primary_bin "generated primary-slot BIN image"
saturn_lab::require_file $primary_prm "generated primary-slot PROM report"
set primary_bin_size [file size $primary_bin]
set primary_slot_capacity [expr {$primary_flash_limit - $primary_flash_base}]
if {$primary_bin_size > $primary_slot_capacity} {
    error "Primary BIN is $primary_bin_size bytes; primary slot capacity is $primary_slot_capacity bytes"
}
set primary_flash_end [expr {$primary_flash_base + $primary_bin_size - 1}]
puts "Primary loader range: [format 0x%08X $primary_flash_base]-[format 0x%08X $primary_flash_end] ($primary_bin_size bytes)"

puts "Generating 32-Mbit SPIx1 image: $output_bin"
puts "  golden  @ 0x00000000: $golden_bit"
puts "  timer1  @ 0x0097FC00: $timer1"
puts "  primary @ 0x00980000: $primary_bit"
puts "  timer2  @ 0x01300000: $timer2"

# No compression option is supplied intentionally: Saturn's multiboot table
# and the existing golden image use uncompressed bitstreams.
write_cfgmem -format bin -size 32 -interface SPIx1 \
    -loadbit [list up 0x00000000 $golden_bit up 0x00980000 $primary_bit] \
    -loaddata [list up 0x0097FC00 $timer1 up 0x01300000 $timer2] \
    $output_bin -force

set prm [file rootname $output_bin].prm
saturn_lab::require_file $output_bin "generated PROM/BIN image"
saturn_lab::require_file $prm "generated PROM report"

set manifest [file join [file dirname $output_bin] prom-manifest.json]
set stream [open $manifest w]
puts $stream "{"
puts $stream "  \"schema\": 2,"
puts $stream "  \"created_utc\": \"[clock format [clock seconds] -gmt true -format {%Y-%m-%dT%H:%M:%SZ}]\","
puts $stream "  \"git_sha\": \"[saturn_lab::json_escape [saturn_lab::git_value rev-parse HEAD]]\","
puts $stream "  \"git_dirty\": [saturn_lab::git_dirty],"
puts $stream "  \"vivado\": \"[saturn_lab::json_escape [version -short]]\","
puts $stream "  \"firmware_version\": 30,"
puts $stream "  \"format\": \"bin\","
puts $stream "  \"interface\": \"SPIx1\","
puts $stream "  \"size_mbit\": 32,"
puts $stream "  \"output\": \"[saturn_lab::json_escape [file tail $output_bin]]\","
puts $stream "  \"output_sha256\": \"[saturn_lab::sha256 $output_bin]\","
puts $stream "  \"primary_bin\": \"[saturn_lab::json_escape [file tail $primary_bin]]\","
puts $stream "  \"primary_bin_sha256\": \"[saturn_lab::sha256 $primary_bin]\","
puts $stream "  \"primary_bin_bytes\": $primary_bin_size,"
puts $stream "  \"loader_destination\": \"[format 0x%08X $primary_flash_base]\","
puts $stream "  \"loader_erase_end\": \"[format 0x%08X $primary_flash_end]\","
puts $stream "  \"primary_bit\": \"[saturn_lab::json_escape [file tail $primary_bit]]\","
puts $stream "  \"primary_sha256\": \"[saturn_lab::sha256 $primary_bit]\","
puts $stream "  \"golden_sha256\": \"[saturn_lab::sha256 $golden_bit]\""
puts $stream "}"
close $stream

puts "SATURN_LAB_PROM_OK primary=$primary_bin combined=$output_bin prm=$prm manifest=$manifest"
