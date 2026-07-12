/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// InDUCIQ.c:
//
// handle "incoming DUC I/Q" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "InDUCIQ.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/byteio.h"
#include "../common/p23_perf_telemetry.h"
#include <pthread.h>
#include <syscall.h>
#include "controller_lease.h"




#define VIQSAMPLESPERFRAME 240                      // samples per UDP frame
#define VMEMWORDSPERFRAME 180                       // memory writes per UDP frame
#define VBYTESPERSAMPLE 6							// 24 bit + 24 bit samples
#define VDMABUFFERSIZE 32768						// memory buffer to reserve
#define VALIGNMENT 4096                             // buffer alignment
#define VBASE 0x1000								// DMA start at 4K into buffer
#define VDMATRANSFERSIZE 1440                       // write 1 message at a time
#define VDUCMAXRECVBATCHFRAMES 8                    // max TX DUC packets drained per socket wakeup
#define VDUCSOFTQUEUEFRAMES 32                      // bounded TX reserve between UDP ingress and XDMA writer
#define VDUCNORMALQUEUEFRAMES 2                     // normal queue depth before a DMA write
#define VDUCPREFILLQUEUEFRAMES 4                    // refill deeper after startup or underrun recovery
#define VMAXDMABATCHFRAMES 11                       // older 2048-word TX FIFOs cannot safely accept larger single bursts
#define VSTARTUPDELAY 100                           // 100 messages (~100ms) before reporting under or overflows
#define VDUCPREFILLLOWFRAMES 3                      // re-enter prefill when FIFO occupancy falls below this
#define VDUCPREFILLHIGHFRAMES 8                     // stay in prefill until FIFO occupancy reaches this
#define VDUCMAXQUEUEAGEUS 1500U                     // cap added TX latency while still allowing occasional coalescing
#define VDUCEMERGENCYLOWFRAMES 2                    // when FIFO occupancy falls this low, stop waiting for deep refill batches
#define VDUCEMERGENCYQUEUEFRAMES 2                  // low-water target to keep the TX DUC FIFO fed
#define VDUCEMERGENCYMAXQUEUEAGEUS 500U             // flush sooner when the TX FIFO is near empty
#define VDUCIDLEPOLLUS 1000U                        // wake periodically while active so FIFO state is refreshed even when no packets arrive
#define VDUCSTALEQUEUEAGEUS 4000U                   // drop oldest queued TX frames once software backlog grows stale

typedef enum
{
    eDUCWriteModeUnknown = 0,
    eDUCWriteModeNormal = 1,
    eDUCWriteModePrefill = 2,
    eDUCWriteModeEmergency = 3
} EDUCWriteMode;

typedef struct
{
    struct ThreadSocketData *ThreadData;
    pthread_mutex_t Mutex;
    pthread_cond_t Cond;
    uint8_t QueueBuffers[VDUCSOFTQUEUEFRAMES][VDMATRANSFERSIZE];
    uint64_t QueueArrivalNs[VDUCSOFTQUEUEFRAMES];
    uint32_t QueueHead;
    uint32_t QueueCount;
    atomic_bool StopRequested;
} TDUCFlowContext;

static void NoteDUCUnderflow(bool ReportingEnabled, bool Underflowed, bool *UnderflowActive,
                             unsigned int Current)
{
    if (!ReportingEnabled)
    {
        *UnderflowActive = false;
        return;
    }

    if (!Underflowed)
    {
        *UnderflowActive = false;
        return;
    }

    pthread_mutex_lock(&g_fifo_overflow_mutex);
    GlobalFIFOOverflows |= 0b00000100;
    pthread_mutex_unlock(&g_fifo_overflow_mutex);

    if (!*UnderflowActive)
    {
        P23PerfTelemetryCounterAdd(eP23PerfCounterFIFODucUnder, 1U);
        *UnderflowActive = true;
    }

    if (UseDebug)
        printf("TX DUC FIFO Underflowed, depth now = %d\n", Current);
}

static uint64_t GetMonotonicTimeNs(void)
{
    struct timespec Now;

    if (clock_gettime(CLOCK_MONOTONIC, &Now) != 0)
        return 0;

    return ((uint64_t)Now.tv_sec * 1000000000ULL) + (uint64_t)Now.tv_nsec;
}

static int DUCQueueTimedWaitLocked(TDUCFlowContext *Context, uint64_t WaitUs)
{
    struct timespec Deadline;

    if (clock_gettime(CLOCK_REALTIME, &Deadline) != 0)
        return -1;

    Deadline.tv_sec += (time_t)(WaitUs / 1000000ULL);
    Deadline.tv_nsec += (long)((WaitUs % 1000000ULL) * 1000ULL);
    if (Deadline.tv_nsec >= 1000000000L)
    {
        Deadline.tv_sec += 1;
        Deadline.tv_nsec -= 1000000000L;
    }

    return pthread_cond_timedwait(&Context->Cond, &Context->Mutex, &Deadline);
}

static void DUCQueueResetLocked(TDUCFlowContext *Context)
{
    Context->QueueHead = 0;
    Context->QueueCount = 0;
}

static void DUCQueueReset(TDUCFlowContext *Context)
{
    pthread_mutex_lock(&Context->Mutex);
    DUCQueueResetLocked(Context);
    pthread_cond_broadcast(&Context->Cond);
    pthread_mutex_unlock(&Context->Mutex);
}

static void DUCQueueSnapshotLocked(const TDUCFlowContext *Context, uint32_t *QueueFrames, uint64_t *QueueAgeUs)
{
    uint64_t NowNs = 0;

    *QueueFrames = Context->QueueCount;
    *QueueAgeUs = 0;
    if (Context->QueueCount == 0U)
        return;

    NowNs = GetMonotonicTimeNs();
    if ((NowNs != 0U) && (Context->QueueArrivalNs[Context->QueueHead] != 0U))
        *QueueAgeUs = (NowNs - Context->QueueArrivalNs[Context->QueueHead]) / 1000ULL;
}

static uint32_t DUCQueueDropStaleLocked(TDUCFlowContext *Context, uint64_t MaxAgeUs)
{
    uint32_t Dropped = 0;
    uint64_t NowNs = GetMonotonicTimeNs();

    if ((NowNs == 0U) || (MaxAgeUs == 0U))
        return 0;

    while (Context->QueueCount > 1U)
    {
        uint64_t ArrivalNs = Context->QueueArrivalNs[Context->QueueHead];
        uint64_t QueueAgeUs = 0;

        if (ArrivalNs == 0U)
            break;
        QueueAgeUs = (NowNs - ArrivalNs) / 1000ULL;
        if (QueueAgeUs <= MaxAgeUs)
            break;

        Context->QueueHead = (Context->QueueHead + 1U) % VDUCSOFTQUEUEFRAMES;
        Context->QueueCount--;
        Dropped++;
    }

    return Dropped;
}

static uint32_t DUCQueuePushFrame(TDUCFlowContext *Context, const uint8_t *FrameData, uint64_t ArrivalNs)
{
    uint32_t QueueTail = 0;
    uint32_t Dropped = 0;

    pthread_mutex_lock(&Context->Mutex);
    if (Context->QueueCount == VDUCSOFTQUEUEFRAMES)
    {
        Context->QueueHead = (Context->QueueHead + 1U) % VDUCSOFTQUEUEFRAMES;
        Context->QueueCount--;
        Dropped = 1U;
    }

    QueueTail = (Context->QueueHead + Context->QueueCount) % VDUCSOFTQUEUEFRAMES;
    memcpy(Context->QueueBuffers[QueueTail], FrameData, VDMATRANSFERSIZE);
    Context->QueueArrivalNs[QueueTail] = ArrivalNs;
    Context->QueueCount++;
    pthread_cond_signal(&Context->Cond);
    pthread_mutex_unlock(&Context->Mutex);

    return Dropped;
}

static uint32_t DUCQueueCopyOutFrames(TDUCFlowContext *Context, uint8_t *DestData, uint32_t MaxFrames,
                                      uint32_t *RemainingFrames, uint64_t *QueueAgeUs)
{
    uint32_t FramesCopied = 0;
    uint32_t FrameIndex = 0;

    pthread_mutex_lock(&Context->Mutex);
    while ((FramesCopied < MaxFrames) && (Context->QueueCount != 0U))
    {
        memcpy(DestData + (FrameIndex * VDMATRANSFERSIZE),
               Context->QueueBuffers[Context->QueueHead],
               VDMATRANSFERSIZE);
        Context->QueueArrivalNs[Context->QueueHead] = 0;
        Context->QueueHead = (Context->QueueHead + 1U) % VDUCSOFTQUEUEFRAMES;
        Context->QueueCount--;
        FramesCopied++;
        FrameIndex++;
    }
    DUCQueueSnapshotLocked(Context, RemainingFrames, QueueAgeUs);
    pthread_mutex_unlock(&Context->Mutex);

    return FramesCopied;
}

static uint32_t GetDUCTargetFrames(unsigned int Current, bool *PrefillActive)
{
    const unsigned int LowWords = VDUCPREFILLLOWFRAMES * VMEMWORDSPERFRAME;
    const unsigned int HighWords = VDUCPREFILLHIGHFRAMES * VMEMWORDSPERFRAME;

    if (Current < LowWords)
        *PrefillActive = true;
    else if (Current >= HighWords)
        *PrefillActive = false;

    return *PrefillActive ? VDUCPREFILLQUEUEFRAMES : VDUCNORMALQUEUEFRAMES;
}

static uint64_t GetDUCQueueAgeLimitUs(unsigned int Current, uint32_t *TargetFrames)
{
    const unsigned int EmergencyWords = VDUCEMERGENCYLOWFRAMES * VMEMWORDSPERFRAME;

    if (Current < EmergencyWords)
    {
        if (*TargetFrames > VDUCEMERGENCYQUEUEFRAMES)
            *TargetFrames = VDUCEMERGENCYQUEUEFRAMES;
        return VDUCEMERGENCYMAXQUEUEAGEUS;
    }

    return VDUCMAXQUEUEAGEUS;
}

static uint32_t GetDUCWriteFrames(unsigned int Current, bool *PrefillActive,
                                  uint32_t QueuedFrames, uint64_t QueueAgeUs)
{
    unsigned int CurrentFrames = Current / VMEMWORDSPERFRAME;
    uint32_t TargetFrames = GetDUCTargetFrames(Current, PrefillActive);
    uint32_t FramesToWrite = 0;

    if (QueuedFrames == 0U)
        return 0;

    if (CurrentFrames < VDUCEMERGENCYLOWFRAMES)
    {
        uint32_t RefillFrames = (CurrentFrames < VDUCPREFILLHIGHFRAMES)
            ? (VDUCPREFILLHIGHFRAMES - CurrentFrames)
            : VDUCEMERGENCYQUEUEFRAMES;

        if ((QueuedFrames >= VDUCEMERGENCYQUEUEFRAMES) || (QueueAgeUs >= VDUCEMERGENCYMAXQUEUEAGEUS))
            FramesToWrite = RefillFrames;
    }
    else if (*PrefillActive)
    {
        uint32_t RefillFrames = (CurrentFrames < VDUCPREFILLHIGHFRAMES)
            ? (VDUCPREFILLHIGHFRAMES - CurrentFrames)
            : TargetFrames;

        if ((QueuedFrames >= TargetFrames) || (QueueAgeUs >= VDUCMAXQUEUEAGEUS))
            FramesToWrite = RefillFrames;
    }
    else if ((QueuedFrames >= TargetFrames) || (QueueAgeUs >= VDUCMAXQUEUEAGEUS))
    {
        FramesToWrite = TargetFrames;
        if (QueuedFrames > (TargetFrames * 2U))
            FramesToWrite = QueuedFrames;
    }

    if (FramesToWrite > QueuedFrames)
        FramesToWrite = QueuedFrames;
    if (FramesToWrite > VMAXDMABATCHFRAMES)
        FramesToWrite = VMAXDMABATCHFRAMES;

    return FramesToWrite;
}

static uint8_t GetDUCWriteMode(unsigned int Current, bool PrefillActive)
{
    unsigned int CurrentFrames = Current / VMEMWORDSPERFRAME;

    if (CurrentFrames < VDUCEMERGENCYLOWFRAMES)
        return eDUCWriteModeEmergency;
    if (PrefillActive)
        return eDUCWriteModePrefill;
    return eDUCWriteModeNormal;
}

static void *DUCIQDMAWriter(void *arg)
{
    TDUCFlowContext *Context = (TDUCFlowContext *)arg;
    uint8_t *IQWriteBuffer = NULL;
    unsigned char *IQBasePtr = NULL;
    uint32_t IQBufferSize = VDMABUFFERSIZE;
    uint32_t Depth = 0;
    int DMAWritefile_fd = -1;
    bool FIFOOverflow = false;
    bool FIFOUnderflow = false;
    bool FIFOOverThreshold = false;
    unsigned int Current = 0;
    unsigned int StartupCount = 0;
    bool PrevSDRActive = false;
    bool UnderflowActive = false;
    bool PrefillActive = true;
    uint32_t QueueCount = 0;
    uint64_t QueueAgeUs = 0;
    uint64_t MaxQueueAgeUs = VDUCMAXQUEUEAGEUS;
    uint32_t TargetFrames = VDUCPREFILLQUEUEFRAMES;
    uint32_t FramesToWrite = 0;
    uint32_t FreeFrames = 0;
    uint32_t WriteBytes = 0;
    uint32_t DroppedFrames = 0;
    uint8_t WriteMode = eDUCWriteModeUnknown;
    bool SDRActiveNow = false;

    printf("spinning up DUC I/Q writer thread, pid=%ld\n", syscall(SYS_gettid));
    ApplyCriticalAudioThreadRuntime("DUC I/Q writer");

    if (posix_memalign((void**)&IQWriteBuffer, VALIGNMENT, IQBufferSize) != 0)
    {
        printf("I/Q TX write buffer allocation failed\n");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    IQBasePtr = IQWriteBuffer + VBASE;
    memset(IQWriteBuffer, 0, IQBufferSize);

    DMAWritefile_fd = open(VDUCDMADEVICE, O_WRONLY);
    if (DMAWritefile_fd < 0)
    {
        perror("XDMA write device open failed for TX I/Q data");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }

    EnableDUCMux(false);
    SetTXIQDeinterleaved(false);
    ResetDUCMux();
    ResetDMAStreamFIFO(eTXDUCDMA);
    SetupFIFOMonitorChannel(eTXDUCDMA, false);
    EnableDUCMux(true);

    while(!atomic_load(&ExitRequested) && !atomic_load(&Context->StopRequested))
    {
        SDRActiveNow = atomic_load(&SDRActive);
        if (SDRActiveNow && !PrevSDRActive)
        {
            StartupCount = VSTARTUPDELAY;
            UnderflowActive = false;
            PrefillActive = true;
        }
        else if(!SDRActiveNow)
        {
            UnderflowActive = false;
            PrefillActive = true;
            Current = 0;
        }
        PrevSDRActive = SDRActiveNow;

        pthread_mutex_lock(&Context->Mutex);
        if ((Context->QueueCount == 0U) && !atomic_load(&Context->StopRequested) && !atomic_load(&ExitRequested))
            (void)DUCQueueTimedWaitLocked(Context, SDRActiveNow ? VDUCIDLEPOLLUS : 5000U);
        DUCQueueSnapshotLocked(Context, &QueueCount, &QueueAgeUs);
        pthread_mutex_unlock(&Context->Mutex);

        Depth = ReadFIFOMonitorChannel(eTXDUCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
        if ((StartupCount == 0U) && FIFOOverThreshold && UseDebug)
            printf("TX DUC FIFO Overthreshold, depth now = %d\n", Current);
        if (FIFOUnderflow)
            PrefillActive = true;
        NoteDUCUnderflow((StartupCount == 0U) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);

        if (QueueCount == 0U)
        {
            P23PerfTelemetrySetDUCQueueContext(0U, Current / VMEMWORDSPERFRAME, 0U, eDUCWriteModeUnknown);
            continue;
        }

        pthread_mutex_lock(&Context->Mutex);
        DroppedFrames = DUCQueueDropStaleLocked(Context, VDUCSTALEQUEUEAGEUS);
        DUCQueueSnapshotLocked(Context, &QueueCount, &QueueAgeUs);
        pthread_mutex_unlock(&Context->Mutex);
        if (DroppedFrames != 0U)
        {
            P23PerfTelemetryCounterAdd(eP23PerfCounterDUCQueueDropEvents, 1U);
            P23PerfTelemetryCounterAdd(eP23PerfCounterDUCQueueDroppedFrames, DroppedFrames);
        }
        if (QueueCount == 0U)
        {
            P23PerfTelemetrySetDUCQueueContext(0U, Current / VMEMWORDSPERFRAME, 0U, eDUCWriteModeUnknown);
            continue;
        }

        TargetFrames = GetDUCTargetFrames(Current, &PrefillActive);
        MaxQueueAgeUs = GetDUCQueueAgeLimitUs(Current, &TargetFrames);
        FramesToWrite = GetDUCWriteFrames(Current, &PrefillActive, QueueCount, QueueAgeUs);
        WriteMode = GetDUCWriteMode(Current, PrefillActive);
        P23PerfTelemetrySetDUCQueueContext(QueueCount, Current / VMEMWORDSPERFRAME,
                                           (QueueAgeUs > UINT32_MAX) ? UINT32_MAX : (uint32_t)QueueAgeUs,
                                           WriteMode);

        if (FramesToWrite == 0U)
        {
            uint64_t WaitUs = 0;

            if (QueueAgeUs < MaxQueueAgeUs)
                WaitUs = MaxQueueAgeUs - QueueAgeUs;
            if ((WaitUs == 0U) || (WaitUs > VDUCIDLEPOLLUS))
                WaitUs = VDUCIDLEPOLLUS;
            usleep((useconds_t)WaitUs);
            continue;
        }

        FreeFrames = Depth / VMEMWORDSPERFRAME;
        while ((FreeFrames == 0U) && !atomic_load(&ExitRequested) && !atomic_load(&Context->StopRequested))
        {
            usleep(500);
            Depth = ReadFIFOMonitorChannel(eTXDUCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
            FreeFrames = Depth / VMEMWORDSPERFRAME;
            if ((StartupCount == 0U) && FIFOOverThreshold && UseDebug)
                printf("TX DUC FIFO Overthreshold, depth now = %d\n", Current);
            if (FIFOUnderflow)
                PrefillActive = true;
            NoteDUCUnderflow((StartupCount == 0U) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);
        }
        if (atomic_load(&ExitRequested) || atomic_load(&Context->StopRequested))
            break;

        if (FramesToWrite > FreeFrames)
            FramesToWrite = FreeFrames;
        if (FramesToWrite == 0U)
            continue;

        FramesToWrite = DUCQueueCopyOutFrames(Context, IQBasePtr, FramesToWrite, &QueueCount, &QueueAgeUs);
        if (FramesToWrite == 0U)
            continue;

        WriteBytes = FramesToWrite * VDMATRANSFERSIZE;
        if (DMAWriteToFPGA(DMAWritefile_fd, IQBasePtr, WriteBytes, VADDRDUCSTREAMWRITE) < 0)
        {
            P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAErrors, 1U);
            atomic_store(&ThreadError, true);
            break;
        }

        P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAWrites, 1U);
        P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAWriteBytes, WriteBytes);
        Current += FramesToWrite * VMEMWORDSPERFRAME;
        if (Current >= (VDUCPREFILLHIGHFRAMES * VMEMWORDSPERFRAME))
            PrefillActive = false;
        P23PerfTelemetrySetDUCQueueContext(QueueCount, Current / VMEMWORDSPERFRAME,
                                           (QueueAgeUs > UINT32_MAX) ? UINT32_MAX : (uint32_t)QueueAgeUs,
                                           WriteMode);
    }

cleanup:
    atomic_store(&Context->StopRequested, true);
    pthread_mutex_lock(&Context->Mutex);
    pthread_cond_broadcast(&Context->Cond);
    pthread_mutex_unlock(&Context->Mutex);
    if (DMAWritefile_fd >= 0)
        close(DMAWritefile_fd);
    free(IQWriteBuffer);
    return NULL;
}

//
// listener thread for incoming DUC I/Q packets
// network ingress and XDMA writing are decoupled by a bounded software queue.
//
void *IncomingDUCIQ(void *arg)                          // listener thread
{
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    TDUCFlowContext *Context = NULL;
    pthread_t WriterThread;
    bool WriterThreadStarted = false;
    uint8_t UDPInBuffers[VDUCMAXRECVBATCHFRAMES][VDUCIQSIZE];
    uint8_t ConvertedFrame[VDMATRANSFERSIZE];
    struct iovec IovecList[VDUCMAXRECVBATCHFRAMES];
    struct mmsghdr DatagramList[VDUCMAXRECVBATCHFRAMES];
    struct sockaddr_in SourceAddresses[VDUCMAXRECVBATCHFRAMES];
    unsigned int MsgIndex = 0;
    int Received = 0;
    uint32_t Cntr = 0;
    uint8_t *SrcPtr = NULL;
    uint8_t *DestPtr = NULL;
    bool PrevSDRActive = false;
    bool SequenceValid = false;
    uint32_t ExpectedSequence = 0;

    ThreadData = (struct ThreadSocketData *)arg;
    Context = calloc(1, sizeof(*Context));
    if (Context == NULL)
    {
        printf("DUC I/Q flow-control context allocation failed\n");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    Context->ThreadData = ThreadData;
    pthread_mutex_init(&Context->Mutex, NULL);
    pthread_cond_init(&Context->Cond, NULL);
    atomic_store(&ThreadData->Active, true);
    printf("spinning up DUC I/Q ingress thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
    ApplyCriticalAudioThreadRuntime("DUC I/Q ingress");

    memset(DatagramList, 0, sizeof(DatagramList));
    memset(SourceAddresses, 0, sizeof(SourceAddresses));
    for (MsgIndex = 0; MsgIndex < VDUCMAXRECVBATCHFRAMES; MsgIndex++)
    {
        IovecList[MsgIndex].iov_base = UDPInBuffers[MsgIndex];
        IovecList[MsgIndex].iov_len = VDUCIQSIZE;
        DatagramList[MsgIndex].msg_hdr.msg_iov = &IovecList[MsgIndex];
        DatagramList[MsgIndex].msg_hdr.msg_iovlen = 1;
        DatagramList[MsgIndex].msg_hdr.msg_name = &SourceAddresses[MsgIndex];
        DatagramList[MsgIndex].msg_hdr.msg_namelen = sizeof(SourceAddresses[MsgIndex]);
    }

    if (pthread_create(&WriterThread, NULL, DUCIQDMAWriter, Context) != 0)
    {
        perror("pthread_create DUC I/Q writer");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    WriterThreadStarted = true;

    while(!atomic_load(&ExitRequested))
    {
        bool SDRActiveNow = atomic_load(&SDRActive);

        if (atomic_load(&Context->StopRequested))
            break;
        if(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT)
        {
            printf("DUC I/Q request change port\n");
            close(GetThreadSocketFD(ThreadData));
            if(MakeSocket(ThreadData, 0) != 0)
            {
                perror("MakeSocket, DUC I/Q");
                atomic_store(&ThreadError, true);
                break;
            }
            atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
            DUCQueueReset(Context);
            SequenceValid = false;
            ExpectedSequence = 0;
        }

        if(SDRActiveNow != PrevSDRActive)
        {
            DUCQueueReset(Context);
            SequenceValid = false;
            ExpectedSequence = 0;
        }
        PrevSDRActive = SDRActiveNow;

        Received = recvmmsg(GetThreadSocketFD(ThreadData), DatagramList, VDUCMAXRECVBATCHFRAMES, MSG_WAITFORONE, NULL);
        if((Received < 0) && (errno != EAGAIN) && (errno != EWOULDBLOCK))
        {
            perror("recvfrom fail, TX I/Q data");
            P23PerfTelemetryCounterAdd(eP23PerfCounterDUCRecvErrors, 1U);
            atomic_store(&ThreadError, true);
            break;
        }
        if(Received < 0)
            Received = 0;

        if(Received > 0)
        {
            for(MsgIndex = 0; MsgIndex < (unsigned int)Received; MsgIndex++)
            {
                uint32_t PacketSequence = 0;
                uint32_t SequenceDelta = 0;
                uint32_t DroppedFrames = 0;
                uint64_t ArrivalNs = 0;

                if(DatagramList[MsgIndex].msg_len != VDUCIQSIZE)
                    continue;
                if(!ControllerLeaseMatches(&SourceAddresses[MsgIndex]))
                    continue;
                atomic_store(&NewMessageReceived, true);
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCPackets, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCBytes, VDUCIQSIZE);
                PacketSequence = rd_be_u32(UDPInBuffers[MsgIndex]);
                if(SequenceValid && (PacketSequence != ExpectedSequence))
                {
                    SequenceDelta = PacketSequence - ExpectedSequence;
                    P23PerfTelemetryCounterAdd(eP23PerfCounterDUCGapEvents, 1U);
                    if((SequenceDelta > 0U) && (SequenceDelta <= 0x7fffffffU))
                        P23PerfTelemetryCounterAdd(eP23PerfCounterDUCGapDroppedFrames, SequenceDelta);
                }
                SequenceValid = true;
                ExpectedSequence = PacketSequence + 1U;

                ArrivalNs = GetMonotonicTimeNs();
                SrcPtr = (uint8_t *) (UDPInBuffers[MsgIndex] + 4);
                DestPtr = ConvertedFrame;
                for (Cntr=0; Cntr < VIQSAMPLESPERFRAME; Cntr++)                     // samplecounter
                {
                    *DestPtr++ = *(SrcPtr+3);                           // get I sample (3 bytes)
                    *DestPtr++ = *(SrcPtr+4);
                    *DestPtr++ = *(SrcPtr+5);
                    *DestPtr++ = *(SrcPtr+0);                           // get Q sample (3 bytes)
                    *DestPtr++ = *(SrcPtr+1);
                    *DestPtr++ = *(SrcPtr+2);
                    SrcPtr += 6;                                        // point at next source sample
                }
                DroppedFrames = DUCQueuePushFrame(Context, ConvertedFrame, ArrivalNs);
                if(DroppedFrames != 0U)
                {
                    P23PerfTelemetryCounterAdd(eP23PerfCounterDUCQueueDropEvents, 1U);
                    P23PerfTelemetryCounterAdd(eP23PerfCounterDUCQueueDroppedFrames, DroppedFrames);
                }
            }
        }
    }

cleanup:
    if (Context != NULL)
    {
        atomic_store(&Context->StopRequested, true);
        pthread_mutex_lock(&Context->Mutex);
        pthread_cond_broadcast(&Context->Cond);
        pthread_mutex_unlock(&Context->Mutex);
    }
    if(WriterThreadStarted)
        pthread_join(WriterThread, NULL);
    if(atomic_load(&ThreadData->Socketid) > 0)
        close(atomic_load(&ThreadData->Socketid));    // close incoming data socket
    atomic_store(&ThreadData->Socketid, 0);
    if (Context != NULL)
    {
        pthread_cond_destroy(&Context->Cond);
        pthread_mutex_destroy(&Context->Mutex);
        free(Context);
    }
    atomic_store(&ThreadData->Active, false);     // indicate it is closed
    return NULL;
}


//
// HandlerSetEERMode (bool EEREnabled)
// enables amplitude restoration mode. Generates envelope output alongside I/Q samples.
// NOTE hardware does not properly support this yet!
// TX FIFO must be empty. Stop multiplexer; set bit; restart
// 
void HandlerSetEERMode(__attribute__((unused)) bool EEREnabled)
{
}
