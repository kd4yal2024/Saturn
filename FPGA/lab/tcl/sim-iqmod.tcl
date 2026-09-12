source [file join [file dirname [file normalize [info script]]] sim-common.tcl]
set key_hold_ns [saturn_lab::env_or SATURN_IQMOD_KEY_HOLD_NS 100000]
set required_samples [saturn_lab::env_or SATURN_IQMOD_SIM_SAMPLES 16]
set simulator_options "-testplusarg SATURN_KEY_HOLD_NS=$key_hold_ns -testplusarg SATURN_REQUIRED_SAMPLES=$required_samples"
saturn_lab::run_simulation \
    [file join $saturn_lab::fpga_dir IP CODEC_IQMOD_IP CODEC_IQMOD_IP.xpr] \
    IQModnCodec_tb iqmod [saturn_lab::env_or SATURN_IQMOD_SIM_RUNTIME all] \
    $simulator_options iqmoddata.txt $required_samples
