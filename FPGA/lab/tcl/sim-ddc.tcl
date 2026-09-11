source [file join [file dirname [file normalize [info script]]] sim-common.tcl]
set required_samples [saturn_lab::env_or SATURN_DDC_SIM_SAMPLES 256]
set discard_samples [saturn_lab::env_or SATURN_DDC_SIM_DISCARD 64]
set simulator_options "-testplusarg SATURN_REQUIRED_SAMPLES=$required_samples -testplusarg SATURN_DISCARD_SAMPLES=$discard_samples"
saturn_lab::run_simulation \
    [file join $saturn_lab::fpga_dir IP DDCIP DDCIP.xpr] \
    rx_ddc_tb ddc [saturn_lab::env_or SATURN_DDC_SIM_RUNTIME all] \
    $simulator_options ddcdata.txt $required_samples
