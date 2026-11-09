/* Offline numerical/ABI positive control. NO radio/device/network access.
 * Build ONLY against the independently source-verified WDSP 2.11 archive.
 * This exercises native fitting/display curves, not G2's installed binary or
 * the production TXA correction/output/FPGA/RF chain.
 */
#define _POSIX_C_SOURCE 200809L
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <fftw3.h>
#include "wdsp.h"
#include "../../docs/puresignal/native-abi.h"

static const uint64_t GUARD = UINT64_C(0x7350477561726421);
typedef struct { uint64_t before; double data[4096]; uint64_t after; } Measured;
typedef struct { uint64_t before; double data[512]; uint64_t after; } Correction;
typedef struct { uint64_t before; double data[2048]; uint64_t after; } Input;
typedef struct { uint64_t before; double data[1024]; uint64_t after; } MicInput;
typedef struct { uint32_t before; int data[16]; uint32_t after; } Info;
typedef struct { uint32_t before; int value; uint32_t after; } Count;
typedef struct { uint64_t before; double value; uint64_t after; } Scalar;
static Measured x, ym, yc, ys;
static Correction xm_cor, ym_cor, xa_cor, ya_cor;
static Input tx, rx;
static MicInput mic;
static Measured duc_output;
static Info info;
static Count measured_count, correction_count;
static Scalar phase;
static uint32_t random_state = 0x53460004;
static uint32_t next_random(void) {
    random_state = random_state * UINT32_C(1664525) + UINT32_C(1013904223);
    return random_state;
}
static void guards_init(void) {
    x.before = x.after = ym.before = ym.after = yc.before = yc.after = ys.before = ys.after = GUARD;
    xm_cor.before = xm_cor.after = ym_cor.before = ym_cor.after = GUARD;
    xa_cor.before = xa_cor.after = ya_cor.before = ya_cor.after = GUARD;
    tx.before = tx.after = rx.before = rx.after = phase.before = phase.after = GUARD;
    mic.before = mic.after = duc_output.before = duc_output.after = GUARD;
    info.before = info.after = measured_count.before = measured_count.after = UINT32_C(0x76543210);
    correction_count.before = correction_count.after = UINT32_C(0x76543210);
}
static int guards_ok(void) {
    Measured *m[] = {&x, &ym, &yc, &ys};
    Correction *c[] = {&xm_cor, &ym_cor, &xa_cor, &ya_cor};
    for (int i = 0; i < 4; ++i) if (m[i]->before != GUARD || m[i]->after != GUARD ||
        c[i]->before != GUARD || c[i]->after != GUARD) return 0;
    return mic.before == GUARD && mic.after == GUARD && duc_output.before == GUARD && duc_output.after == GUARD &&
        tx.before == GUARD && tx.after == GUARD && rx.before == GUARD && rx.after == GUARD &&
        phase.before == GUARD && phase.after == GUARD &&
        info.before == UINT32_C(0x76543210) && info.after == UINT32_C(0x76543210) &&
        measured_count.before == UINT32_C(0x76543210) && measured_count.after == UINT32_C(0x76543210) &&
        correction_count.before == UINT32_C(0x76543210) && correction_count.after == UINT32_C(0x76543210);
}
static void fill_nonlinear_pair(void) {
    const double pi = acos(-1.0);
    for (int i = 0; i < 1024; ++i) {
        double r = 0.001 + 0.999 * (double)next_random() / 4294967296.0;
        double angle = 2.0 * pi * (double)next_random() / 4294967296.0;
        /* Known memoryless AM/AM compression and AM/PM rotation. Full-scale
         * reference=0.6121, feedback=0.375. Both returned arrays are normalized
         * sample magnitudes, NOT watts or calibrated physical ADC levels. */
        double feedback = 0.5 * r * (1.0 - 0.25 * r * r);
        double phase_error = 0.18 * r * r;
        tx.data[2*i] = 0.6121 * r * cos(angle);
        tx.data[2*i+1] = 0.6121 * r * sin(angle);
        rx.data[2*i] = feedback * cos(angle + phase_error);
        rx.data[2*i+1] = feedback * sin(angle + phase_error);
    }
}
static void get_display(void) {
    GetPSDisp(1, x.data, ym.data, yc.data, ys.data, xm_cor.data, ym_cor.data,
        xa_cor.data, ya_cor.data, &measured_count.value, &correction_count.value, &phase.value);
}
static void pump_dsp(void) {
    int error = 0;
    // Native correction start/swap/end waits for xiqc to acknowledge its ramp.
    // Running pscc alone with TXA stopped can leave a successful fit in state6.
    // Zero mic exchanges advance that DSP handshake; output goes ONLY to memory.
    fexchange0(1, mic.data, duc_output.data, &error);
}
int main(void) {
    if (GetWDSPVersion() != 211) { fprintf(stderr, "wrong native version\n"); return 2; }
    // Bound offline startup planning only; no altered WDSP numerical sources.
    fftw_set_timelimit(0.01);
    guards_init();
    OpenChannel(1, 512, 2048, 48000, 96000, 192000, 1, 0, .01, .025, 0, .01, 1);
    SetChannelState(1, 1, 0);
    SetPSMox(1, 0); SetPSControl(1, 1, 0, 0, 0);
    SetPSFeedbackRate(1, 192000); SetPSHWPeak(1, .6121);
    double actual_delay = SetPSTXDelay(1, 150e-9);
    if (!isfinite(actual_delay) || fabs(actual_delay - 150e-9) > 21e-9) {
        fprintf(stderr, "bad native quantized delay %.12g\n", actual_delay); return 3;
    }
    SetPSTXDelay(1, 0); SetPSMoxDelay(1, 0); SetPSLoopDelay(1, 0);
    for (int i = 0; i < 8; ++i) { pscc(1, 1024, tx.data, rx.data); pump_dsp(); }
    get_display(); if (!guards_ok()) { fprintf(stderr, "initial ABI canary failed\n"); return 4; }
    SetPSControl(1, 0, 1, 0, 0); SetPSMox(1, 1);
    int blocks;
    const struct timespec pause = { .tv_sec = 0, .tv_nsec = 1000000 };
    for (blocks = 0; blocks < 2048; ++blocks) {
        fill_nonlinear_pair(); pscc(1, 1024, tx.data, rx.data);
        pump_dsp();
        GetPSInfo(1, info.data); get_display();
        if (!guards_ok()) { fprintf(stderr, "ABI canary failed at block %d\n", blocks); return 5; }
        if (info.data[5] > 0 && info.data[14] != 0) break;
        nanosleep(&pause, NULL); // let native asynchronous fitter run, no RF waits
    }
    SetPSMox(1, 0);
    if (blocks == 2048 || (info.data[6] & 3) || measured_count.value != 4096 || correction_count.value != 512) {
        fprintf(stderr, "native fit incomplete: blocks=%d calibrations=%d correcting=%d status=%d state=%d measured=%d correction=%d\n",
            blocks, info.data[5], info.data[14], info.data[6], info.data[15], measured_count.value, correction_count.value);
        SetChannelState(1, 0, 1); CloseChannel(1); return 6;
    }
    double uncorrected = 0, corrected = 0; int compared = 0;
    for (int i = 0; i < correction_count.value; ++i) {
        double target = xm_cor.data[i]; double gain = ym_cor.data[i];
        if (!isfinite(target) || !isfinite(gain) || !isfinite(ya_cor.data[i])) {
            fprintf(stderr, "nonfinite correction curve\n"); SetChannelState(1, 0, 1); CloseChannel(1); return 7;
        }
        if (target < .1 || target > .9) continue;
        double drive = target * gain;
        double y_without = target * (1 - .25 * target * target) / .75;
        double y_with = drive * (1 - .25 * drive * drive) / .75;
        uncorrected += (y_without - target) * (y_without - target);
        corrected += (y_with - target) * (y_with - target); ++compared;
    }
    if (!compared || corrected >= 0.1 * uncorrected) {
        fprintf(stderr, "native curve did not improve synthetic AM/AM residual: %.9g -> %.9g\n", uncorrected, corrected);
        SetChannelState(1, 0, 1); CloseChannel(1); return 8;
    }
    printf("native positive control passed: version=211 blocks=%d calibrations=%d measured=%d correction=%d canaries=pass requested_delay_ns=150 actual_delay_ns=%.6f modeled_uncorrected_rms=%.9g modeled_corrected_rms=%.9g\n",
        blocks + 1, info.data[5], measured_count.value, correction_count.value, actual_delay * 1e9,
        sqrt(uncorrected / compared), sqrt(corrected / compared));
    SetPSControl(1, 1, 0, 0, 0);
    for (int i = 0; i < 8; ++i) { pscc(1, 1024, tx.data, rx.data); pump_dsp(); }
    SetChannelState(1, 0, 0);
    SetChannelState(1, 0, 1); CloseChannel(1); return 0;
}
