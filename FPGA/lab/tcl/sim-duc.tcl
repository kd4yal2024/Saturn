source [file join [file dirname [file normalize [info script]]] sim-common.tcl]
set required_samples [saturn_lab::env_or SATURN_DUC_SIM_SAMPLES 4096]
set discard_samples [saturn_lab::env_or SATURN_DUC_SIM_DISCARD 16384]
set simulator_options "-testplusarg SATURN_REQUIRED_SAMPLES=$required_samples -testplusarg SATURN_DISCARD_SAMPLES=$discard_samples"
saturn_lab::run_simulation \
    [file join $saturn_lab::fpga_dir IP DUCIP DUCIP.xpr] \
    tx_duc_tb duc [saturn_lab::env_or SATURN_DUC_SIM_RUNTIME all] \
    $simulator_options ducoffbindata.txt $required_samples
