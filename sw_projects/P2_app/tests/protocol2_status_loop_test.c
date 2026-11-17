/* Actual status packing/wait loop with deterministic hardware, time and send
 * boundaries. No device, UDP socket, filesystem export or wall-time claim. */
#define clock_gettime FixtureClockGettime
#define usleep FixtureSleep
#define sendmsg FixtureSendmsg
#define access FixtureAccess
#ifndef P2_STATUS_LOOP_SOURCE
#define P2_STATUS_LOOP_SOURCE "../OutHighPriority.c"
#endif
#include P2_STATUS_LOOP_SOURCE
#undef clock_gettime
#undef usleep
#undef sendmsg
#undef access

#include <assert.h>

atomic_bool SDRActive, ExitRequested, IsTXMode, ReplyAddressSet;
atomic_bool StartBitReceived, ThreadError;
bool MOXAsserted; /* permits the same fixture to reproduce the old implementation */
struct sockaddr_in reply_addr;
pthread_mutex_t g_reply_addr_mutex = PTHREAD_MUTEX_INITIALIZER;

enum Scenario { RX, TX, PTT_EVENT, ADC_EVENT, TX_ONSET, LATE_WORK, BAD_CLOCK };
static enum Scenario Scenario;
static uint64_t NowNs, WorkNs, Sent[4];
static unsigned int Count, Sleeps;
static uint8_t Packets[4][60];
static bool ADCFired, Closed;

int FixtureClockGettime(clockid_t Clock, struct timespec *Now)
{
    assert(Clock == CLOCK_MONOTONIC);
    if(Scenario == BAD_CLOCK) { errno = EIO; return -1; }
    Now->tv_sec = (time_t)(NowNs / 1000000000ULL);
    Now->tv_nsec = (long)(NowNs % 1000000000ULL);
    return 0;
}
int FixtureSleep(useconds_t Microseconds)
{
    assert(++Sleeps < 10000U); /* catches a stalled/busy test loop */
    assert(Microseconds > 0 && Microseconds <= 500U);
    NowNs += (uint64_t)Microseconds * 1000U;
    if(Scenario == TX_ONSET && NowNs >= 10000000ULL)
    {
        atomic_store(&IsTXMode, true);
        MOXAsserted = true;
    }
    return 0;
}
int FixtureAccess(const char *Path, int Mode)
{
    assert(strcmp(Path, ADC_PEAK_TELEMETRY_ENABLE_FILE) == 0);
    assert(Mode == F_OK);
    errno = ENOENT;
    return -1; /* never enable real file export */
}
ssize_t FixtureSendmsg(int Socket, const struct msghdr *Message, int Flags)
{
    assert(Socket == 7 && Flags == 0 && Count < 4);
    assert(Message->msg_iovlen == 1 && Message->msg_iov[0].iov_len == 60);
    memcpy(Packets[Count], Message->msg_iov[0].iov_base, 60);
    Sent[Count] = NowNs;
    assert(rd_be_u32(Packets[Count]) == Count);
    ++Count;
    if(Count == 4) atomic_store(&ExitRequested, true);
    return 60;
}

int GetThreadSocketFD(const struct ThreadSocketData *Data)
{ (void)Data; return 7; }
bool CloseThreadSocketIfOwned(const struct ThreadSocketData *Data)
{ (void)Data; Closed = true; return true; }
void ReadStatusRegister(void) {}
unsigned int GetP2PTTKeyInputs(void)
{ return Scenario == PTT_EVENT && NowNs >= 10000000ULL ? 1U : 0U; }
unsigned int GetADCOverflow(uint16_t *ADC1, uint16_t *ADC2)
{
    *ADC1 = 1234; *ADC2 = 2345;
    if(Scenario == ADC_EVENT && NowNs >= 7000000ULL && !ADCFired)
    { ADCFired = true; return 1; }
    return 0;
}
unsigned int GetAnalogueIn(unsigned int Index) { return Index; }
unsigned int GetUserIOBits(void) { return 0; }
bool GetPureSignalEnabled(void) { return false; }
float GetDieTempC(void) { return 40.; }
uint32_t ReadFIFOMonitorChannel(EDMAStreamSelect Channel, bool *Over, bool *Threshold,
                              bool *Under, unsigned int *Current)
{
    (void)Channel;
    *Over = *Threshold = *Under = false; *Current = 0; return 0;
}
void RequestATUTune(bool Requested) { (void)Requested; }
bool FPGAFifoV29MaybeSample(void) { return false; }
bool FPGAADCV30MaybeSample(void) { return false; }
void RXC1MaybeSample(void) {}
void FPGAFifoV29GetSnapshot(TFPGAFifoV29Snapshot *Snapshot)
{ memset(Snapshot, 0, sizeof(*Snapshot)); }
void FPGAADCV30GetSnapshot(TFPGAADCV30Snapshot *Snapshot)
{ memset(Snapshot, 0, sizeof(*Snapshot)); }
void RXC1GetSnapshot(TRXC1Snapshot *Snapshot)
{ memset(Snapshot, 0, sizeof(*Snapshot)); }
void P23PerfTelemetrySetRuntimeFlags(bool A, bool B, bool C, bool D, bool E, bool F)
{ (void)A; (void)B; (void)C; (void)D; (void)E; (void)F; }
void P23PerfTelemetrySetPureSignalEnabled(bool Enabled) { (void)Enabled; }
void P23PerfTelemetrySetDieTempC(float Temp) { (void)Temp; }
void P23PerfTelemetrySetFPGAFifoV29(const TFPGAFifoV29Snapshot *S) { (void)S; }
void P23PerfTelemetrySetFPGAADCV30(const TFPGAADCV30Snapshot *S) { (void)S; }
void P23PerfTelemetrySetRXCounterV31(const TRXC1Snapshot *S) { (void)S; }
void P23PerfTelemetrySetFIFOSnapshot(uint32_t A, uint32_t B, uint32_t C, uint32_t D, uint8_t E)
{ (void)A; (void)B; (void)C; (void)D; (void)E; }
void P23PerfTelemetrySetADCSnapshot(uint16_t A, uint16_t B, uint8_t C)
{ (void)A; (void)B; (void)C; }
void P23PerfTelemetryCounterAdd(EP23PerfCounterId Counter, uint64_t Delta)
{ (void)Counter; (void)Delta; }
void P23PerfTelemetryMaybeWrite(void)
{
    NowNs += WorkNs;
    if(Scenario == LATE_WORK && Count == 1) NowNs += 2500000ULL;
}

static void Run(enum Scenario Which, uint64_t Work, const uint64_t Expected[4])
{
    struct ThreadSocketData Data = {0};
    Scenario = Which; WorkNs = Work; NowNs = Count = Sleeps = 0;
    ADCFired = Closed = false;
    atomic_store(&SDRActive, true); atomic_store(&ExitRequested, false);
    atomic_store(&ThreadError, false);
    atomic_store(&IsTXMode, Which == TX || Which == LATE_WORK);
    MOXAsserted = atomic_load(&IsTXMode);
    OutgoingHighPriority(&Data);
    assert(Closed && !atomic_load(&Data.Active));
    if(Which == BAD_CLOCK)
    { assert(Count == 0 && atomic_load(&ThreadError)); return; }
    assert(!atomic_load(&ThreadError) && Count == 4);
    for(unsigned int Index = 0; Index < 4; ++Index)
    {
        if(Sent[Index] != Expected[Index])
            fprintf(stderr, "scenario=%u packet=%u at=%llu expected=%llu\n", Which, Index,
                    (unsigned long long)Sent[Index], (unsigned long long)Expected[Index]);
        assert(Sent[Index] == Expected[Index]);
        assert(rd_be_u16(Packets[Index] + 39) == 1234);
        assert(rd_be_u16(Packets[Index] + 41) == 2345);
        assert(Packets[Index][5] == (Which == ADC_EVENT && Index == 1 ? 1 : 0));
    }
}

int main(void)
{
    Run(RX, 3000000ULL, (uint64_t[]){0, 50000000ULL, 100000000ULL, 150000000ULL});
    Run(TX, 300000ULL, (uint64_t[]){0, 1000000ULL, 2000000ULL, 3000000ULL});
    Run(PTT_EVENT, 0, (uint64_t[]){0, 10000000ULL, 50000000ULL, 100000000ULL});
    assert(Packets[0][4] == 0 && Packets[1][4] == 1);
    Run(ADC_EVENT, 0, (uint64_t[]){0, 7000000ULL, 50000000ULL, 100000000ULL});
    Run(TX_ONSET, 0, (uint64_t[]){0, 10000000ULL, 11000000ULL, 12000000ULL});
    Run(LATE_WORK, 0, (uint64_t[]){0, 2500000ULL, 3000000ULL, 4000000ULL});
    Run(BAD_CLOCK, 0, NULL);
    puts("protocol2 status loop tests passed (synthetic time, no hardware)");
    return 0;
}
