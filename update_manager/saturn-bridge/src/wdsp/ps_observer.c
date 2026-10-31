/* Build-matched observation only. No radio access or WDSP state mutation.
 * build.rs rejects archives/headers other than the reviewed native builds.
 * The single TX/native owner must keep channel 1 alive during these calls.
 */
#include "comm.h"
#include "extrapolate.h"
#include "saturn-calcc-layout.h"

/* Nonblocking copy of the exact GetPSDisp buffers (WDSP Guide 2.1 pp109-110).
 * Sole native owner only: channel recreation must never run concurrently.
 * This observer neither invokes fitting nor changes any native control.
 * Return 0 for contention, -1 for incompatible capacities, 1 for a snapshot.
 * Unlike GetPSDisp, this never waits behind the calibration thread. */
int saturn_try_ps_display(int channel, double* x, double* ym, double* yc, double* ys,
    double* xm, double* mc, double* xa, double* ac, int* ns, int* nc,
    double* reference, int* info, int* flags) {
    if (pthread_mutex_trylock(&txa[channel].calcc.cs_update)) return 0;
    CALCC a = txa[channel].calcc.p;
    IQC iq = txa[channel].iqc.p;
    if (!a || !iq) {
        LeaveCriticalSection(&txa[channel].calcc.cs_update);
        return -1;
    }
    if (pthread_mutex_trylock(&a->disp.cs_disp)) {
        LeaveCriticalSection(&txa[channel].calcc.cs_update);
        return 0;
    }
    int n = a->disp.nsamps;
    if (n < 0 || n > 4096) {
        LeaveCriticalSection(&a->disp.cs_disp);
        LeaveCriticalSection(&txa[channel].calcc.cs_update);
        return -1;
    }
    memcpy(x, a->disp.x, n * sizeof(double));
    memcpy(ym, a->disp.ym, n * sizeof(double));
    memcpy(yc, a->disp.yc, n * sizeof(double));
    memcpy(ys, a->disp.ys, n * sizeof(double));
    memcpy(xm, a->disp.xm_cor, 512 * sizeof(double));
    memcpy(mc, a->disp.ym_cor, 512 * sizeof(double));
    memcpy(xa, a->disp.xa_cor, 512 * sizeof(double));
    memcpy(ac, a->disp.ya_cor, 512 * sizeof(double));
    *ns = n; *nc = 512; *reference = a->disp.phs_ref_deg;
    memcpy(info, a->info, 16 * sizeof(int));
    *flags = !!__atomic_load_n(&iq->run, __ATOMIC_SEQ_CST) |
        (!!__atomic_load_n(&iq->busy, __ATOMIC_SEQ_CST) << 1);
    LeaveCriticalSection(&a->disp.cs_disp);
    LeaveCriticalSection(&txa[channel].calcc.cs_update);
    return 1;
}

/* Sole serialized TX owner; no DSP exchange or second channel owner. */
double saturn_shared_tx_mic_gain(void) {
    return txa[1].panel.p ? txa[1].panel.p->gain1 : -1.0;
}

/* Sole serialized native owner. Readback only: no generation, key or DSP call. */
int saturn_shared_generator_readback(double* values) {
    GEN g = txa[1].gen1.p;
    if (!g) return 0;
    values[0] = g->run; values[1] = g->mode;
    values[2] = g->tt.mag1; values[3] = g->tt.mag2;
    values[4] = g->tt.f1; values[5] = g->tt.f2;
    return 1;
}

/* Build-matched observation of delay routing and reference scaling. Sole
 * native owner only. Never key, fit, change delay, or start an exchange. */
int saturn_shared_ps_delay_readback(double* tx_delay, double* rf_delay, double* reference_peak) {
    CALCC a = txa[1].calcc.p;
    if (!a) return 0;
    EnterCriticalSection(&txa[1].calcc.cs_update);
    *tx_delay = a->txdelay->adelay;
    *rf_delay = a->rxdelay->adelay;
    *reference_peak = 1.0 / a->hw_scale;
    LeaveCriticalSection(&txa[1].calcc.cs_update);
    return 1;
}

/* Readback, not configured intent. Return zero only when manual/automatic
 * fitting, native MOX, restored-curve activation, in-progress fit, IQC run and
 * IQC busy are all disabled. Sole owner keeps the channel alive. */
int saturn_shared_measurement_flags(void) {
    CALCC a = txa[1].calcc.p;
    IQC iq = txa[1].iqc.p;
    if (!a || !iq) return -1;
    EnterCriticalSection(&txa[1].calcc.cs_update);
    int flags = !!a->ctrl.mancal | (!!a->ctrl.automode << 1) |
        (!!a->ctrl.turnon << 2) | (!!a->ctrl.calcinprogress << 3) |
        (!!__atomic_load_n(&a->mox, __ATOMIC_SEQ_CST) << 4) |
        (!!__atomic_load_n(&iq->run, __ATOMIC_SEQ_CST) << 5) |
        (!!__atomic_load_n(&iq->busy, __ATOMIC_SEQ_CST) << 6);
    LeaveCriticalSection(&txa[1].calcc.cs_update);
    return flags;
}

_Static_assert(sizeof(txa[1].iqc.p->run) == sizeof(long), "ported IQC run width");
_Static_assert(sizeof(txa[1].iqc.p->busy) == sizeof(long), "ported IQC busy width");

int saturn_shared_iqc_flags(void) {
    IQC a = txa[1].iqc.p;
    if (!a) return -1;
    long run = __atomic_load_n(&a->run, __ATOMIC_SEQ_CST);
    long busy = __atomic_load_n(&a->busy, __ATOMIC_SEQ_CST);
    if ((run != 0 && run != 1) || (busy != 0 && busy != 1)) return -1;
    return run | (busy << 1);
}

int saturn_shared_satp_history(void) {
    RMATCH a = (RMATCH)create_rmatchV(64, 512, 48000, 48000, 4096, 1.0);
    if (!a) return -1;
    int history = a->v ? a->v->rsize : -1;
    if (a->insize != 64 || a->outsize != 512 || a->nom_inrate != 48000 ||
        a->nom_outrate != 48000 || a->rsize != 4096) history = -1;
    destroy_rmatchV(a);
    return history;
}
