/* Real-library regression; synthetic buffers only, no device/network access. */
#include "wdsp.h"
#include "wdsp_align.h"
#include <assert.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#ifdef SATURN_TEST_NNR_PROFILE
extern void reset_profile_nnet(void);
extern void report_profile_nnet(void);
#endif

static double seconds(void) {
    struct timespec t;
    assert(!clock_gettime(CLOCK_MONOTONIC, &t));
    return t.tv_sec + t.tv_nsec * 1e-9;
}

int main(void) {
    assert(GetWDSPVersion() == 211);
    for (size_t alignment = 4; alignment <= 64; alignment *= 2) {
        void *p = _aligned_malloc(513, alignment);
        assert(p && (uintptr_t)p % alignment == 0);
        memset(p, 0xa5, 513);
        _aligned_free(p);
    }
    char long_path[1100];
    memset(long_path, 'a', sizeof(long_path) - 1);
    long_path[sizeof(long_path) - 1] = 0;
    assert(WDSPwisdom(NULL) == -1);
    assert(WDSPwisdom(long_path) == -1);

    double input[1024], output[1024];
    uint64_t sample = 0;
    OpenChannel(0, 512, 2048, 48000, 48000, 48000, 0, 1,
                0, 0, 0, 0, 1);
    SetRXABandpassFreqs(0, 200, 3000);
    SetRXANNRRun(0, 1);
    for (int model = 0; model < 4; model++) {
        SetRXAMode(0, model >= 2 ? RXA_CWU : RXA_USB);
        assert(SetRXANNRModel(0, model) == model);
        assert(GetRXANNRModel(0) == model);
#ifdef SATURN_TEST_NNR_PROFILE
        reset_profile_nnet();
#endif
        double elapsed = 0, maximum = 0, energy = 0;
        for (int block = 0; block < 520; block++) {
            for (int i = 0; i < 512; i++, sample++) {
                double phase = 6.283185307179586 * 700 * sample / 48000;
                input[2*i] = .01 * cos(phase);
                input[2*i+1] = .01 * sin(phase);
            }
            int error = 0;
            double start = seconds();
            fexchange0(0, input, output, &error);
            double duration = seconds() - start;
            elapsed += duration;
            if (duration > maximum) maximum = duration;
            assert(!error);
            for (int i = 0; i < 1024; i++) {
                assert(isfinite(output[i]));
                energy += output[i] * output[i];
            }
        }
        assert(energy > 0);
        printf("WDSP211 model=%d finite_audio=yes energy=%.9g mean_exchange_ms=%.6f max_exchange_ms=%.6f synthetic_audio_seconds=%.6f\n",
               model, energy, elapsed * 1000 / 520, maximum * 1000, 520. * 512 / 48000);
#ifdef SATURN_TEST_NNR_PROFILE
        report_profile_nnet();
        char name[64];
        snprintf(name, sizeof(name), "nnr-profile-model-%d.txt", model);
        assert(!rename("nnr_profile.txt", name));
#endif
    }
    assert(SetRXANNRModel(0, 4) == 3);
    SetRXANNRRun(0, 0);
    SetChannelState(0, 0, 0);
    memset(input, 0, sizeof(input));
    for (int i = 0; i < 32; i++) { int error; fexchange0(0, input, output, &error); }
    SetChannelState(0, 0, 1);
    CloseChannel(0);
    puts("WDSP211 version/alignment/wisdom/four-model/close contracts passed; RF untested");
}
