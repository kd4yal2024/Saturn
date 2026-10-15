/* Offline diagnostic ONLY. Same pinned/ported headers as the linked archive.
 * Read two existing atomic IQC flags; do not change WDSP algorithms or state.
 * The single Rust owner keeps channel 1 alive through every call.
 */
#include "comm.h"

int saturn_offline_iqc_flags(void) {
    IQC a = txa[1].iqc.p;
    return (int)(_InterlockedAnd(&a->run, 1) | (_InterlockedAnd(&a->busy, 1) << 1));
}

int saturn_offline_satp_history(void) {
    void *port = create_rmatchV(64, 512, 48000, 48000, 4096, 1.0);
    RMATCH a = (RMATCH)port;
    int history = a->v->rsize;
    if (a->insize != 64 || a->outsize != 512 || a->nom_inrate != 48000 ||
        a->nom_outrate != 48000 || a->rsize != 4096) history = -1;
    destroy_rmatchV(port);
    return history;
}

/* Read actual native settings under the single serialized channel owner. */
int saturn_offline_tx_features(void) {
    return txa[1].eqp.p->run | (txa[1].cfcomp.p->run << 1) |
        (txa[1].phrot.p->run << 2) | (txa[1].compressor.p->run << 3) |
        (txa[1].osctrl.p->run << 4) | (txa[1].amsq.p->run << 5);
}
double saturn_offline_tx_mic_gain(void) { return txa[1].panel.p->gain1; }
