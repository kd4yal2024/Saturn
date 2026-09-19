/* Isolated equivalence and timing harness; no radio/device access. */
#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "reshb.h"

extern HBResampler opt_create_HBResampler(uint32_t, uint32_t, uint32_t, complex_t *, complex_t *);
extern void opt_xHBResampler(HBResampler);
extern void opt_flush_HBResampler(HBResampler);
extern void opt_destroy_HBResampler(HBResampler);

static uint32_t rng = 17;
static double noise(void) {
    rng = rng * 1664525u + 1013904223u;
    return ((double)rng / 4294967296.0) * 2.0 - 1.0;
}
static double seconds(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}
static void compare(HBResampler a, HBResampler b, int count) {
    assert(a->nStages == b->nStages);
    assert(memcmp(a->out, b->out, count * sizeof(complex_t)) == 0);
    for (unsigned s = 0; s < a->nStages; ++s) {
        assert(a->rsmps[s].ring_ptr == b->rsmps[s].ring_ptr);
        assert(memcmp(a->rsmps[s].ring, b->rsmps[s].ring,
                      a->rsmps[s].N * sizeof(complex_t)) == 0);
    }
}
static void check_rate(unsigned rate, unsigned output, int block, int in_place) {
    complex_t *in = calloc(block, sizeof(*in)), *copy = calloc(block, sizeof(*copy));
    complex_t *out = calloc(block, sizeof(*out)), *opt = calloc(block, sizeof(*opt));
    assert(in && copy && out && opt);
    HBResampler a = create_HBResampler(rate, output, block, in, in_place ? in : out);
    HBResampler b = opt_create_HBResampler(rate, output, block, copy, in_place ? copy : opt);
    int count = a->run ? block / (rate / output) : block;
    for (int pattern = 0; pattern < 6; ++pattern) {
        flush_HBResampler(a);
        opt_flush_HBResampler(b);
        for (int frame = 0; frame < 32; ++frame) {
            for (int j = 0; j < block; ++j) {
                int n = frame * block + j;
                double x = pattern == 0 ? 0 : pattern == 1 ? (n == 0) :
                    pattern == 2 ? 0.999999 : pattern == 3 ? noise() :
                    sin(n * (pattern == 4 ? 0.01 : 2.9));
                in[j] = (complex_t){x, pattern == 3 ? noise() : -x};
            }
            memcpy(copy, in, block * sizeof(*in));
            xHBResampler(a);
            opt_xHBResampler(b);
            compare(a, b, count);
        }
    }
    destroy_HBResampler(a); opt_destroy_HBResampler(b);
    free(in); free(copy); free(out); free(opt);
}
static void benchmark(void) {
    const int block = 2048, repeats = 4000;
    complex_t in[2048], out[2048], opt[2048];
    for (int i = 0; i < block; ++i) in[i] = (complex_t){noise(), noise()};
    HBResampler a = create_HBResampler(384000, 48000, block, in, out);
    HBResampler b = opt_create_HBResampler(384000, 48000, block, in, opt);
    for (int i = 0; i < 100; ++i) { xHBResampler(a); opt_xHBResampler(b); }
    for (int trial = 0; trial < 4; ++trial) {
        double elapsed[2];
        for (int pass = 0; pass < 2; ++pass) {
            int which = (trial + pass) % 2;
            double start = seconds();
            for (int i = 0; i < repeats; ++i)
                if (which) opt_xHBResampler(b); else xHBResampler(a);
            elapsed[which] = seconds() - start;
        }
        compare(a, b, block / 8);
        printf("384k->48k trial=%d reference_ms=%.3f candidate_ms=%.3f speedup=%.2fx\n",
               trial, elapsed[0] * 1000, elapsed[1] * 1000, elapsed[0] / elapsed[1]);
    }
    destroy_HBResampler(a); opt_destroy_HBResampler(b);
}
int main(void) {
    unsigned rates[] = {96000, 192000, 384000, 768000, 1536000, 3072000, 6144000};
    int cases = 0;
    for (unsigned i = 0; i < sizeof(rates)/sizeof(rates[0]); ++i)
        for (unsigned output = 48000; output <= 192000; output *= 2)
            if (rates[i] >= output)
                for (int block = 256; block <= 2048; block *= 8)
                    for (int alias = 0; alias < 2; ++alias) {
                        check_rate(rates[i], output, block, alias);
                        ++cases;
                    }
    printf("PASS bit-identical output/ring state: %d rate/block/alias cases, six signals, 32 blocks each\n", cases);
    benchmark();
    return 0;
}
