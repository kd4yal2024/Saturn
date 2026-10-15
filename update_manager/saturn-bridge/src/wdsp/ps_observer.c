/* Build-matched observation only. No radio access or WDSP state mutation.
 * build.rs rejects archives/headers other than the reviewed native builds.
 * The single TX/native owner must keep channel 1 alive during these calls.
 */
#include "comm.h"
#include "extrapolate.h"
#include "saturn-calcc-layout.h"

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
