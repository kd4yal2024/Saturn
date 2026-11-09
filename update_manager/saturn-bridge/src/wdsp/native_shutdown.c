/* Included after the build-matched comm.h. The port's CloseChannel waits a
 * fixed 25 ms before freeing DSP memory. A scheduled-out worker can outlive
 * that delay. Keep the shared TX channel (1) worker joinable and require
 * actual termination. RX rate changes and other WDSP workers retain the
 * library's existing creation semantics. */
#include <errno.h>
#include <stdlib.h>

typedef struct {
    pthread_t thread;
    int present, joined, stopping, channel;
} SATURN_CHANNEL_WORKER;
static SATURN_CHANNEL_WORKER saturn_workers[MAX_CHANNELS];
static pthread_mutex_t saturn_workers_mutex = PTHREAD_MUTEX_INITIALIZER;

extern HANDLE __real__beginthread(void (*)(void *), unsigned, void *);
extern void __real_CloseChannel(int);

static void *saturn_channel_main(void *arg) {
    SATURN_CHANNEL_WORKER *worker = arg;
    char name[16];
    snprintf(name, sizeof(name), "Wchan%d", worker->channel);
    pthread_setname_np(pthread_self(), name);
    wdspmain((void *)(uintptr_t)worker->channel);
    return NULL;
}

HANDLE __wrap__beginthread(void (*start)(void *), unsigned stack_size, void *arg) {
    if (start != wdspmain || (uintptr_t)arg != 1)
        return __real__beginthread(start, stack_size, arg);
    int channel = (int)(uintptr_t)arg;
    if (channel < 0 || channel >= MAX_CHANNELS) return (HANDLE)-1;
    pthread_mutex_lock(&saturn_workers_mutex);
    SATURN_CHANNEL_WORKER *worker = &saturn_workers[channel];
    if (worker->present && !worker->joined) {
        pthread_mutex_unlock(&saturn_workers_mutex);
        return (HANDLE)-1;
    }
    pthread_attr_t attr;
    int rc = pthread_attr_init(&attr);
    if (rc) {
        pthread_mutex_unlock(&saturn_workers_mutex);
        return (HANDLE)-1;
    }
    if (!rc && stack_size) rc = pthread_attr_setstacksize(&attr, stack_size);
    if (!rc) {
        worker->channel = channel;
        worker->joined = worker->stopping = 0;
        rc = pthread_create(&worker->thread, &attr, saturn_channel_main, worker);
        worker->present = rc == 0;
    }
    pthread_attr_destroy(&attr);
    HANDLE result = rc ? (HANDLE)-1 : (HANDLE)worker->thread;
    pthread_mutex_unlock(&saturn_workers_mutex);
    return result;
}

/* Sole channel owner calls this after completed output stop and fitter/IQC
 * quiescence. No DSP storage is freed here. Return 0 while still in flight,
 * 1 only after joining the worker, -1 for an invalid/unverified state. */
int saturn_channel_shutdown_poll(int channel) {
    if (channel != 1) return -1;
    pthread_mutex_lock(&saturn_workers_mutex);
    SATURN_CHANNEL_WORKER *worker = &saturn_workers[channel];
    if (!worker->present) {
        pthread_mutex_unlock(&saturn_workers_mutex);
        return -1;
    }
    if (worker->joined) {
        pthread_mutex_unlock(&saturn_workers_mutex);
        return 1;
    }
    if (!worker->stopping) {
        /* WDSP 2.11 requires a completed OFF transition before CloseChannel.
         * Begin it without blocking this lifecycle turn; the native owner
         * continues zero-input exchanges while shutdown is pending. */
        if (ch[channel].state != 0) {
            SetChannelState(channel, 0, 0);
            pthread_mutex_unlock(&saturn_workers_mutex);
            return 0;
        }
        if (__atomic_load_n(&ch[channel].flushflag, __ATOMIC_SEQ_CST) & 1) {
            pthread_mutex_unlock(&saturn_workers_mutex);
            return 0;
        }
        SetChannelState(channel, 0, 1); /* Already flushed: completes immediately. */
        /* dexchange exits its pthread when run is clear. Only clear it while
         * holding csDSP, so a worker cannot exit with that mutex still held.
         * A worker already waiting for csDSP observes exec_bypass after us. */
        int busy = pthread_mutex_trylock(&ch[channel].csDSP);
        if (busy) {
            pthread_mutex_unlock(&saturn_workers_mutex);
            return busy == EBUSY ? 0 : -1;
        }
        InterlockedBitTestAndReset(&ch[channel].exchange, 0);
        InterlockedBitTestAndReset(&ch[channel].run, 0);
        InterlockedBitTestAndSet(&ch[channel].iob.pc->exec_bypass, 0);
        ReleaseSemaphore(ch[channel].iob.pc->Sem_BuffReady, 1, 0);
        worker->stopping = 1;
        pthread_mutex_unlock(&ch[channel].csDSP);
    }
    int rc = pthread_tryjoin_np(worker->thread, NULL);
    if (!rc) worker->joined = 1;
    pthread_mutex_unlock(&saturn_workers_mutex);
    return !rc ? 1 : rc == EBUSY ? 0 : -1;
}

/* Managed cleanup polls first, so it reaches the close only after the join.
 * This build's legacy TX destruction also waits; RX is unchanged. */
void __wrap_CloseChannel(int channel) {
    /* Managed cleanup has completed the OFF/flush stage already. Other close
     * paths use WDSP's bounded wait before destroying the channel. */
    SetChannelState(channel, 0, 1);
    if (channel != 1) { __real_CloseChannel(channel); return; }
    int state;
    while ((state = saturn_channel_shutdown_poll(channel)) == 0) usleep(1000);
    if (state != 1) abort(); /* Never free a channel whose worker is unverified. */
    __real_CloseChannel(channel);
}
