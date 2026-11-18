/* Exercise the real ingress loops across RUN=0/reclaim entirely inside one
 * mocked recv wake. No DMA device, real socket, thread or RF is used. */
#define recvmmsg FixtureReceive
#define open FixtureOpen
#define close FixtureClose
#define clock_gettime FixtureClock
#ifdef TEST_DUC
#define pthread_create FixtureCreate
#define pthread_join FixtureJoin
#ifndef P2_STREAM_SOURCE
#define P2_STREAM_SOURCE "../InDUCIQ.c"
#endif
#else
#ifndef P2_STREAM_SOURCE
#define P2_STREAM_SOURCE "../InSpkrAudio.c"
#endif
#endif
#include P2_STREAM_SOURCE
#undef recvmmsg
#undef open
#undef close
#undef clock_gettime
#ifdef TEST_DUC
#undef pthread_create
#undef pthread_join
#endif
#include <assert.h>
#include <arpa/inet.h>

atomic_bool SDRActive, ExitRequested, ThreadError, NewMessageReceived;
bool UseDebug;
uint8_t GlobalFIFOOverflows;
pthread_mutex_t g_fifo_overflow_mutex = PTHREAD_MUTEX_INITIALIZER;
static struct sockaddr_in Source;
static unsigned int Receives, Writes;
#ifdef TEST_DUC
static TDUCFlowContext *Flow;
int FixtureCreate(pthread_t *Thread, const pthread_attr_t *Attr,
                  void *(*Start)(void *), void *Arg)
{
    (void)Attr; (void)Start; *Thread = (pthread_t)1; Flow = Arg; return 0;
}
int FixtureJoin(pthread_t Thread, void **Result)
{
    (void)Thread; (void)Result;
    assert(Flow->QueueCount == 3); /* old epoch's queued frame was discarded */
    assert(Flow->QueueBuffers[Flow->QueueHead][0] == 0x55);
    assert(Flow->QueueBuffers[(Flow->QueueHead + 2) % VDUCSOFTQUEUEFRAMES][0] == 0x99);
    return 0;
}
#endif
int FixtureOpen(const char *Path, int Flags, ...) { (void)Path; (void)Flags; return 9; }
int FixtureClose(int FD) { (void)FD; return 0; }
int FixtureClock(clockid_t Clock, struct timespec *Now)
{ static long Tick; (void)Clock; Now->tv_sec = 1; Now->tv_nsec = (++Tick) * 1000000L; return 0; }
int GetThreadSocketFD(const struct ThreadSocketData *Data) { (void)Data; return 10; }
bool CloseThreadSocketIfOwned(const struct ThreadSocketData *Data) { (void)Data; return true; }
int MakeSocket(struct ThreadSocketData *Data, int ID) { (void)Data; (void)ID; return 0; }
void ApplyCriticalAudioThreadRuntime(const char *Name) { (void)Name; }
uint32_t GetFIFOStatus(uint32_t Channel, bool *Overflow, bool *OverThreshold,
                       bool *Underflow, unsigned int *Current)
{
    (void)Channel; *Overflow = *OverThreshold = *Underflow = false; *Current = 0; return 1024;
}
int DMAWriteToFPGA(int FD, unsigned char *Buffer, unsigned int Length, uint32_t Address)
{
    (void)FD; (void)Address; (void)Buffer; (void)Length;
#ifndef TEST_DUC
    if(Receives == 1) { assert(Length == VDMATRANSFERSIZE && Buffer[0] == 0x33); }
    else {
        assert(Receives == 2); assert(Length == 3 * VDMATRANSFERSIZE);
        assert(Buffer[0] == 0x55 && Buffer[2 * VDMATRANSFERSIZE] == 0x99);
        atomic_store(&ExitRequested, true);
    }
#endif
    ++Writes; return 0;
}
void ResetDMAStreamFIFO(EDMAStreamSelect Channel) { (void)Channel; }
void SetupFIFOMonitorChannel(EDMAStreamSelect Channel, bool Enable) { (void)Channel; (void)Enable; }
uint32_t ReadFIFOMonitorChannel(EDMAStreamSelect Channel, bool *Overflow,
                               bool *Threshold, bool *Underflow, unsigned int *Current)
{
    (void)Channel; *Overflow = *Threshold = *Underflow = false; *Current = 0;
    return 1024;
}
int FixtureReceive(int FD, struct mmsghdr *Messages, unsigned int Count, int Flags,
                    struct timespec *Timeout)
{
    (void)FD; (void)Flags; (void)Timeout; assert(++Receives <= 3); assert(Count >= 6);
    if(Receives == 3) { atomic_store(&ExitRequested, true); return 0; }
    unsigned int N = Receives == 1 ? 1 : 6;
    if(Receives == 2)
    {
        /* Both flags change before either ingress loop can poll them. */
        atomic_store(&SDRActive, false);
        assert(ControllerLeaseRelease(&Source));
        assert(ControllerLeaseClaim(&Source));
        atomic_store(&SDRActive, true);
#ifdef TEST_DUC
        atomic_store(&ExitRequested, true);
#endif
    }
    const uint32_t Sequences[] = {0, 1, 1, 0, 2, 3};
    const uint8_t Values[] = {0x55, 0x66, 0x77, 0x88, 0x99, 0xaa};
    for(unsigned int I = 0; I < N; ++I)
    {
        uint8_t *Data = Messages[I].msg_hdr.msg_iov[0].iov_base;
        size_t Size = Messages[I].msg_hdr.msg_iov[0].iov_len;
        uint32_t Sequence = htonl(Receives == 1 ? 1000 : Sequences[I]);
        memset(Data, Receives == 1 ? 0x33 : Values[I], Size);
        memcpy(Data, &Sequence, sizeof(Sequence));
        Messages[I].msg_len = Size; Messages[I].msg_hdr.msg_flags = 0;
        memcpy(Messages[I].msg_hdr.msg_name, &Source, sizeof(Source));
        if(I == 5) ((struct sockaddr_in *)Messages[I].msg_hdr.msg_name)->sin_addr.s_addr ^= 1;
    }
    return (int)N;
}
int main(void)
{
    struct ThreadSocketData Data = {0};
    Source.sin_family = AF_INET; Source.sin_addr.s_addr = htonl(0xc0000201);
    assert(ControllerLeaseClaim(&Source)); atomic_store(&SDRActive, true);
#ifdef TEST_DUC
    IncomingDUCIQ(&Data);
#else
    IncomingSpkrAudio(&Data); assert(Writes == 2);
#endif
    assert(Receives == 2 && !atomic_load(&ThreadError));
    ControllerLeaseClear();
    puts("real ingress fast-restart, stale/duplicate and controller isolation tests passed");
    return 0;
}
