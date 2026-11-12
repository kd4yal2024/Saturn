/* Deterministic pthread regression: a native worker outlives the old 25 ms
 * grace period. The production shutdown wrapper must not free its channel. */
#include <assert.h>
#include <pthread.h>
#include <semaphore.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>
#define HANDLE void *
#define MAX_CHANNELS 4
#define InterlockedBitTestAndReset(p,b) __sync_fetch_and_and(p,~(1L<<(b)))
#define InterlockedBitTestAndSet(p,b) __sync_fetch_and_or(p,1L<<(b))
typedef struct { volatile long exec_bypass; sem_t *Sem_BuffReady; } Iob;
static struct { int state; volatile long run, exchange, flushflag; pthread_mutex_t csDSP; struct { Iob *pc; } iob; } ch[MAX_CHANNELS];
static int off_waits;
static int SetChannelState(int channel, int state, int dmode) {
    assert(state == 0);
    int prior = ch[channel].state;
    ch[channel].state = 0;
    if (prior) ch[channel].flushflag = 1;
    if (dmode) { assert(!ch[channel].flushflag); off_waits++; }
    return prior;
}
static void ReleaseSemaphore(sem_t *sem, int n, int unused) { (void)unused; while(n--) assert(!sem_post(sem)); }
static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;
static int entered, may_exit, finished, frees, delegated;
void wdspmain(void *arg) {
    int channel = (int)(uintptr_t)arg;
    pthread_mutex_lock(&ch[channel].csDSP);
    pthread_mutex_lock(&gate);
    entered = 1; pthread_cond_broadcast(&changed);
    while(!may_exit) pthread_cond_wait(&changed, &gate);
    pthread_mutex_unlock(&gate);
    pthread_mutex_unlock(&ch[channel].csDSP);
    assert(!sem_wait(ch[channel].iob.pc->Sem_BuffReady));
    pthread_mutex_lock(&gate);
    assert(ch[channel].run == 0 && ch[channel].iob.pc->exec_bypass == 1);
    finished = 1;
    pthread_mutex_unlock(&gate);
}
HANDLE __real__beginthread(void (*start)(void *), unsigned stack, void *arg) {
    (void)start; (void)stack; (void)arg; delegated++; return (HANDLE)42;
}
void __real_CloseChannel(int channel) { assert(!ch[channel].state && !ch[channel].flushflag && off_waits); assert(finished); frees++; }
#include "../../update_manager/saturn-bridge/src/wdsp/native_shutdown.c"
static void unrelated(void *arg) { (void)arg; }
static void cycle(int channel, int wait_for_entry) {
    sem_t sem; assert(!sem_init(&sem, 0, 0)); Iob io = {0, &sem};
    assert(!pthread_mutex_init(&ch[channel].csDSP,NULL));
    ch[channel].iob.pc=&io; ch[channel].state=1; ch[channel].run=ch[channel].exchange=1;
    entered=may_exit=finished=0; int before=frees;
    assert(__wrap__beginthread(wdspmain,0,(void *)(uintptr_t)channel)!=(HANDLE)-1);
    if(wait_for_entry) {
        pthread_mutex_lock(&gate); while(!entered) pthread_cond_wait(&changed,&gate); pthread_mutex_unlock(&gate);
    }
    assert(saturn_channel_shutdown_poll(channel)==0);
    /* Remain paused beyond the native CloseChannel's historical 25 ms. */
    usleep(50000);
    for(int i=0;i<50;i++) assert(saturn_channel_shutdown_poll(channel)==0);
    if(wait_for_entry) assert(ch[channel].run == 1);
    assert(frees==before);
    /* The sole owner advances zero input until WDSP's flush completes. */
    assert(!ch[channel].state && ch[channel].flushflag);
    assert(!saturn_channel_flush_complete(channel));
    ch[channel].flushflag=0;
    pthread_mutex_lock(&gate); may_exit=1; pthread_cond_broadcast(&changed); pthread_mutex_unlock(&gate);
    int joined=0;
    for(int i=0;i<10000 && !joined;i++) { int s=saturn_channel_shutdown_poll(channel); assert(s>=0); joined=s; if(!joined) usleep(100); }
    assert(joined==1 && finished); assert(saturn_channel_flush_complete(channel)); __wrap_CloseChannel(channel); assert(frees==before+1);
    assert(!pthread_mutex_destroy(&ch[channel].csDSP));
    assert(!sem_destroy(&sem));
}
int main(void) {
    assert(!saturn_channel_flush_complete(-1));
    assert(!saturn_channel_flush_complete(1));
    assert(saturn_channel_shutdown_poll(-1)==-1);
    assert(saturn_channel_shutdown_poll(1)==-1);
    assert(__wrap__beginthread(unrelated,0,NULL)==(HANDLE)42 && delegated==1);
    cycle(1,1); cycle(1,0);
    assert(__wrap__beginthread(wdspmain,0,NULL)==(HANDLE)42 && delegated==2);
    puts("WDSP channel shutdown: delayed worker retained, joined before free, fresh channel reopened");
}
