/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// InSpkrAudio.c:
//
// handle "incoming speaker audio" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "InSpkrAudio.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <pthread.h>
#include <sys/socket.h>
#include <time.h>
#include <syscall.h>
#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/byteio.h"
#include "../common/p23_perf_telemetry.h"


#define VSPKSAMPLESPERFRAME 64                      // samples per UDP frame
#define VMEMWORDSPERFRAME 32                        // 8 byte writes per UDP msg
#define VSPKSAMPLESPERMEMWORD 2                     // 2 samples (each 4 bytres) per 8 byte word
#define VDMABUFFERSIZE 32768						// memory buffer to reserve
#define VALIGNMENT 4096                             // buffer alignment
#define VBASE 0x1000								// DMA start at 4K into buffer
#define VDMATRANSFERSIZE 256                        // write 1 message at a time
#define VSPKNORMALQUEUEFRAMES 3                     // normal queue depth before a DMA write
#define VSPKPREFILLQUEUEFRAMES 6                    // minimum frames to prefer when rebuilding speaker reserve
#define VSPKMAXRECVBATCHFRAMES 8                    // max speaker packets we read per socket wakeup
#define VSPKSOFTQUEUEFRAMES 32                      // software reserve to absorb jitter across wakeups
#define VSPKMAXDMAWRITEFRAMES 12                    // max speaker frames we coalesce into one DMA write
#define VSTARTUPDELAY 100                           // 100 messages (~100ms) before reporting under or overflows
#define VSPKPREFILLLOWFRAMES 6                      // re-enter prefill when speaker FIFO reserve drops below this
#define VSPKPREFILLHIGHFRAMES 16                    // refill toward this reserve before leaving prefill mode
#define VSPKMAXQUEUEAGEUS 3000U                     // bound speaker latency while still allowing reserve building
#define VSPKMAXRECVWAITUS 500U                      // re-check FIFO state frequently once speaker data is queued
#define VSPKIDLEPOLLUS 1000U                        // wake periodically while active so speaker gaps do not hide behind a blocking recv
#define VSPKSTALLDETECTUS 20000U                    // only treat a prolonged empty receive queue as a true speaker stream stall
#define VSPKEMERGENCYLOWFRAMES 4                    // treat speaker FIFO below this as an emergency refill case
#define VSPKEMERGENCYQUEUEFRAMES 2                  // minimum frames to flush once speaker FIFO is near empty
#define VSPKEMERGENCYMAXQUEUEAGEUS 750U             // do not let queued speaker audio age longer in emergency mode
#define VSPKGAPSILENCELOWFRAMES 4                   // once a stream gap is active, keep at least this many silence frames queued in hardware
#define VSPKGAPSILENCEHIGHFRAMES 6                  // refill only a small silence cushion so resumed audio is not delayed too much
#define VSPKGAPSILENCEMAXWRITEFRAMES 4              // bound per-write silence injection during a stream gap
#define VSPKMAXSEQUENCEGAPSILENCE 8                 // bridge short speaker packet gaps in-order without flushing the reserve queue

typedef enum
{
    eSpeakerWriteModeUnknown = 0,
    eSpeakerWriteModeNormal = 1,
    eSpeakerWriteModePrefill = 2,
    eSpeakerWriteModeEmergency = 3,
    eSpeakerWriteModeGapFill = 4
} ESpeakerWriteMode;

static void NoteSpeakerUnderflow(bool ReportingEnabled, bool Underflowed, bool *UnderflowActive,
                                 unsigned int Current, uint32_t QueueFrames, uint64_t QueueAgeUs,
                                 bool PrefillIsActive, bool GapIsActive)
{
    uint8_t Mode = eSpeakerWriteModeUnknown;

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
    GlobalFIFOOverflows |= 0b00001000;
    pthread_mutex_unlock(&g_fifo_overflow_mutex);

    if (!*UnderflowActive)
    {
        P23PerfTelemetryCounterAdd(eP23PerfCounterFIFOSpkrUnder, 1U);
        if (QueueFrames == 0U)
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrUnderQueueEmpty, 1U);
        else
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrUnderQueueReady, 1U);

        if (GapIsActive)
            Mode = eSpeakerWriteModeGapFill;
        else if ((Current / VMEMWORDSPERFRAME) < VSPKEMERGENCYLOWFRAMES)
            Mode = eSpeakerWriteModeEmergency;
        else if (PrefillIsActive)
            Mode = eSpeakerWriteModePrefill;
        else
            Mode = eSpeakerWriteModeNormal;

        P23PerfTelemetrySetSpeakerUnderrunContext(
            QueueFrames,
            Current / VMEMWORDSPERFRAME,
            (QueueAgeUs > UINT32_MAX) ? UINT32_MAX : (uint32_t)QueueAgeUs,
            Mode,
            GapIsActive
        );
        *UnderflowActive = true;
    }

    if (UseDebug)
        printf("Codec speaker FIFO Underflowed, depth now = %d\n", Current);
}

static uint64_t GetMonotonicTimeNs(void)
{
    struct timespec Now;

    if (clock_gettime(CLOCK_MONOTONIC, &Now) != 0)
        return 0;

    return ((uint64_t)Now.tv_sec * 1000000000ULL) + (uint64_t)Now.tv_nsec;
}

static uint32_t GetSpeakerTargetFrames(unsigned int Current, bool *PrefillActive)
{
    const unsigned int LowWords = VSPKPREFILLLOWFRAMES * VMEMWORDSPERFRAME;
    const unsigned int HighWords = VSPKPREFILLHIGHFRAMES * VMEMWORDSPERFRAME;

    if (Current < LowWords)
        *PrefillActive = true;
    else if (Current >= HighWords)
        *PrefillActive = false;

    return *PrefillActive ? VSPKPREFILLQUEUEFRAMES : VSPKNORMALQUEUEFRAMES;
}

static uint32_t GetSpeakerWriteFrames(unsigned int Current, bool *PrefillActive,
                                      uint32_t QueuedFrames, uint64_t QueueAgeUs)
{
    unsigned int CurrentFrames = Current / VMEMWORDSPERFRAME;
    uint32_t TargetFrames = GetSpeakerTargetFrames(Current, PrefillActive);
    uint32_t FramesToWrite = 0;

    if (QueuedFrames == 0)
        return 0;

    if (CurrentFrames < VSPKEMERGENCYLOWFRAMES)
    {
        uint32_t RefillFrames = (CurrentFrames < VSPKPREFILLHIGHFRAMES)
            ? (VSPKPREFILLHIGHFRAMES - CurrentFrames)
            : VSPKEMERGENCYQUEUEFRAMES;

        if ((QueuedFrames >= VSPKEMERGENCYQUEUEFRAMES) || (QueueAgeUs >= VSPKEMERGENCYMAXQUEUEAGEUS))
            FramesToWrite = RefillFrames;
    }
    else if (*PrefillActive)
    {
        uint32_t RefillFrames = (CurrentFrames < VSPKPREFILLHIGHFRAMES)
            ? (VSPKPREFILLHIGHFRAMES - CurrentFrames)
            : TargetFrames;

        if ((QueuedFrames >= TargetFrames) || (QueueAgeUs >= VSPKMAXQUEUEAGEUS))
            FramesToWrite = RefillFrames;
    }
    else if ((QueuedFrames >= TargetFrames) || (QueueAgeUs >= VSPKMAXQUEUEAGEUS))
    {
        FramesToWrite = TargetFrames;
        if (QueuedFrames > (TargetFrames * 2U))
            FramesToWrite = QueuedFrames;
    }

    if (FramesToWrite > QueuedFrames)
        FramesToWrite = QueuedFrames;
    if (FramesToWrite > VSPKMAXDMAWRITEFRAMES)
        FramesToWrite = VSPKMAXDMAWRITEFRAMES;

    return FramesToWrite;
}

static uint32_t GetSpeakerGapSilenceFrames(unsigned int Current, uint32_t FreeFrames)
{
    unsigned int CurrentFrames = Current / VMEMWORDSPERFRAME;
    uint32_t FramesToWrite = 0;

    if (CurrentFrames >= VSPKGAPSILENCELOWFRAMES)
        return 0;

    if (CurrentFrames < VSPKGAPSILENCEHIGHFRAMES)
        FramesToWrite = VSPKGAPSILENCEHIGHFRAMES - CurrentFrames;

    if (FramesToWrite > FreeFrames)
        FramesToWrite = FreeFrames;
    if (FramesToWrite > VSPKGAPSILENCEMAXWRITEFRAMES)
        FramesToWrite = VSPKGAPSILENCEMAXWRITEFRAMES;

    return FramesToWrite;
}

//
// listener thread for incoming DDC (speaker) audio packets
// planned strategy: just DMA spkr data when available; don't copy and DMA a larger amount.
// if sufficient FIFO data available: DMA that data and transfer it out. 
// if it turns out to be too inefficient, we'll have to try larger DMA.
//
void *IncomingSpkrAudio(void *arg)                      // listener thread
{
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    uint8_t UDPInBuffers[VSPKMAXRECVBATCHFRAMES][VSPEAKERAUDIOSIZE];
    uint8_t QueueBuffers[VSPKSOFTQUEUEFRAMES][VDMATRANSFERSIZE];
    uint64_t QueueArrivalNs[VSPKSOFTQUEUEFRAMES];
    struct iovec IovecList[VSPKMAXRECVBATCHFRAMES];
    struct mmsghdr DatagramList[VSPKMAXRECVBATCHFRAMES];
    unsigned int MsgIndex = 0;
    unsigned int FrameIndex = 0;
    int Received = 0;

//
// variables for DMA buffer 
//
    uint8_t* SpkWriteBuffer = NULL;							// data for DMA to write to spkr
    uint32_t SpkBufferSize = VDMABUFFERSIZE;
    unsigned char* SpkBasePtr;								// ptr to DMA location in spk memory
    uint32_t Depth = 0;
    int DMAWritefile_fd = -1;								// DMA read file device
    bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;
    uint32_t RegVal = 0;
    unsigned int Current = 0;                               // current occupied locations in FIFO
    unsigned int StartupCount = 0;                          // used to delay reporting of under & overflows
    bool PrevSDRActive = false;                             // used to detect change of state
    bool UnderflowActive = false;                           // tracks one continuous starvation episode
    bool PrefillActive = true;                              // maintains a speaker FIFO cushion after startup or underrun
    uint32_t QueueHead = 0;
    uint32_t QueueCount = 0;
    uint32_t FramesToWrite = 0;
    uint32_t SilenceFramesToWrite = 0;
    uint32_t WriteBytes = 0;
    uint64_t QueueAgeUs = 0;
    uint64_t RemainingAgeUs = 0;
    uint64_t LastPacketNs = 0;
    uint64_t NowNs = 0;
    bool GapMuteActive = false;
    bool SequenceValid = false;
    uint32_t ExpectedSequence = 0;
    struct timespec ReceiveTimeout;
    struct timespec *ReceiveTimeoutPtr = NULL;


    ThreadData = (struct ThreadSocketData *)arg;
    atomic_store(&ThreadData->Active, true);
    printf("spinning up speaker audio thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
    ApplyCriticalAudioThreadRuntime("Speaker audio");

    memset(DatagramList, 0, sizeof(DatagramList));
    memset(QueueArrivalNs, 0, sizeof(QueueArrivalNs));
    for (MsgIndex = 0; MsgIndex < VSPKMAXRECVBATCHFRAMES; MsgIndex++)
    {
        IovecList[MsgIndex].iov_base = UDPInBuffers[MsgIndex];
        IovecList[MsgIndex].iov_len = VSPEAKERAUDIOSIZE;
        DatagramList[MsgIndex].msg_hdr.msg_iov = &IovecList[MsgIndex];
        DatagramList[MsgIndex].msg_hdr.msg_iovlen = 1;
    }

    //
    // setup DMA buffer
    //
    if (posix_memalign((void**)&SpkWriteBuffer, VALIGNMENT, SpkBufferSize) != 0)
    {
        SpkWriteBuffer = NULL;
        printf("spkr write buffer allocation failed\n");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    SpkBasePtr = SpkWriteBuffer + VBASE;
    memset(SpkWriteBuffer, 0, SpkBufferSize);

    //
    // open DMA device driver
    // opened write only to accommodate potential use of a different XDMA device driver
    //
    DMAWritefile_fd = open(VSPKDMADEVICE, O_WRONLY);
    if (DMAWritefile_fd < 0)
    {
        perror("XDMA write device open failed for spk data");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    ResetDMAStreamFIFO(eSpkCodecDMA);
    SetupFIFOMonitorChannel(eSpkCodecDMA, false);

  //
  // main processing loop
  // modified to have the same structure as outgoing threads; capable of being stopped and started.
  //
    while(!atomic_load(&ExitRequested))
    {
        if(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT)
        {
            printf("Speaker audio request change port\n");
            close(GetThreadSocketFD(ThreadData));
            if(MakeSocket(ThreadData, 0) != 0)
            {
                perror("MakeSocket, Speaker audio");
                atomic_store(&ThreadError, true);
                break;
            }
            atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
        }

        //
        // now released to start processing. Setup buffers.
        //
        bool SDRActiveNow = atomic_load(&SDRActive);
        if(SDRActiveNow && !PrevSDRActive)                  // detect SDRActive has been asserted
        {
            StartupCount = VSTARTUPDELAY;
            UnderflowActive = false;
            PrefillActive = true;
            QueueHead = 0;
            QueueCount = 0;
            LastPacketNs = 0;
            GapMuteActive = false;
            SequenceValid = false;
            ExpectedSequence = 0;
            memset(QueueArrivalNs, 0, sizeof(QueueArrivalNs));
        }
        else if(!SDRActiveNow)
        {
            UnderflowActive = false;
            PrefillActive = true;
            Current = 0;
            QueueHead = 0;
            QueueCount = 0;
            LastPacketNs = 0;
            GapMuteActive = false;
            SequenceValid = false;
            ExpectedSequence = 0;
            memset(QueueArrivalNs, 0, sizeof(QueueArrivalNs));
        }
        PrevSDRActive = SDRActiveNow;

        if(QueueCount < VSPKSOFTQUEUEFRAMES)
        {
            unsigned int ReceiveGoal = VSPKSOFTQUEUEFRAMES - QueueCount;
            int ReceiveFlags = MSG_WAITFORONE;

            if(ReceiveGoal > VSPKMAXRECVBATCHFRAMES)
                ReceiveGoal = VSPKMAXRECVBATCHFRAMES;
            ReceiveTimeoutPtr = NULL;
            if(QueueCount != 0)
            {
                QueueAgeUs = 0;
                if(QueueArrivalNs[QueueHead] != 0)
                    QueueAgeUs = (GetMonotonicTimeNs() - QueueArrivalNs[QueueHead]) / 1000ULL;
                if(QueueAgeUs < VSPKMAXQUEUEAGEUS)
                    RemainingAgeUs = VSPKMAXQUEUEAGEUS - QueueAgeUs;
                else
                    RemainingAgeUs = 0;
                if(RemainingAgeUs == 0)
                    Received = 0;
                else
                {
                    if(RemainingAgeUs > VSPKMAXRECVWAITUS)
                        RemainingAgeUs = VSPKMAXRECVWAITUS;
                    ReceiveTimeout.tv_sec = (time_t)(RemainingAgeUs / 1000000ULL);
                    ReceiveTimeout.tv_nsec = (long)((RemainingAgeUs % 1000000ULL) * 1000ULL);
                    ReceiveTimeoutPtr = &ReceiveTimeout;
                    Received = recvmmsg(atomic_load(&ThreadData->Socketid), DatagramList, ReceiveGoal, ReceiveFlags, ReceiveTimeoutPtr);
                }
            }
            else if(SDRActiveNow)
            {
                ReceiveTimeout.tv_sec = 0;
                ReceiveTimeout.tv_nsec = (long)(VSPKIDLEPOLLUS * 1000ULL);
                ReceiveTimeoutPtr = &ReceiveTimeout;
                Received = recvmmsg(atomic_load(&ThreadData->Socketid), DatagramList, ReceiveGoal, ReceiveFlags, ReceiveTimeoutPtr);
            }
            else
                Received = recvmmsg(atomic_load(&ThreadData->Socketid), DatagramList, ReceiveGoal, ReceiveFlags, NULL);

            if((Received < 0) && (errno != EAGAIN) && (errno != EWOULDBLOCK))
            {
                perror("recvfrom fail, Speaker data");
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrRecvErrors, 1U);
                atomic_store(&ThreadError, true);
                break;
            }
            if(Received < 0)
                Received = 0;
        }
        else
            Received = 0;

        if(Received > 0)                                        // we have received one or more packets
        {
            for(MsgIndex = 0; MsgIndex < (unsigned int)Received; MsgIndex++)
            {
                uint32_t QueueTail = 0;
                uint32_t PacketSequence = 0;
                uint32_t SequenceDelta = 0;
                uint32_t MissingFrames = 0;
                uint32_t InsertedFrames = 0;
                uint32_t SilenceFrames = 0;
                uint32_t FreeFrames = 0;
                uint64_t PacketArrivalNs = 0;

                if(DatagramList[MsgIndex].msg_len != VSPEAKERAUDIOSIZE)
                    continue;
                if(QueueCount >= VSPKSOFTQUEUEFRAMES)
                    break;
                if(StartupCount != 0)                                   // decrement startup message count
                    StartupCount--;
                atomic_store(&NewMessageReceived, true);
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrPackets, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrBytes, VSPEAKERAUDIOSIZE);
                RegVal += 1;            //debug
                PacketSequence = rd_be_u32(UDPInBuffers[MsgIndex]);
                PacketArrivalNs = GetMonotonicTimeNs();
                if(SequenceValid && (PacketSequence != ExpectedSequence))
                {
                    SequenceDelta = PacketSequence - ExpectedSequence;
                    P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrGapEvents, 1U);
                    PrefillActive = true;

                    if((SequenceDelta > 0U) && (SequenceDelta <= 0x7fffffffU))
                    {
                        MissingFrames = SequenceDelta;
                        if(MissingFrames > VSPKMAXSEQUENCEGAPSILENCE)
                            MissingFrames = VSPKMAXSEQUENCEGAPSILENCE;
                        FreeFrames = VSPKSOFTQUEUEFRAMES - QueueCount;
                        if(FreeFrames > 0U)
                            FreeFrames--;
                        SilenceFrames = (MissingFrames < FreeFrames) ? MissingFrames : FreeFrames;
                        while(SilenceFrames != 0U)
                        {
                            QueueTail = (QueueHead + QueueCount) % VSPKSOFTQUEUEFRAMES;
                            memset(QueueBuffers[QueueTail], 0, VDMATRANSFERSIZE);
                            QueueArrivalNs[QueueTail] = PacketArrivalNs;
                            QueueCount++;
                            SilenceFrames--;
                            InsertedFrames++;
                        }
                        if(InsertedFrames != 0U)
                            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrSilenceFrames, InsertedFrames);
                        if(SequenceDelta > InsertedFrames)
                            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrGapDroppedFrames, (uint64_t)(SequenceDelta - InsertedFrames));
                    }
                }
                QueueTail = (QueueHead + QueueCount) % VSPKSOFTQUEUEFRAMES;
                memcpy(QueueBuffers[QueueTail], UDPInBuffers[MsgIndex] + 4, VDMATRANSFERSIZE);     // store speaker samples in the software reserve
                QueueArrivalNs[QueueTail] = PacketArrivalNs;
                LastPacketNs = PacketArrivalNs;
                QueueCount++;
                GapMuteActive = false;
                SequenceValid = true;
                ExpectedSequence = PacketSequence + 1U;
            }
        }
        else if(SDRActiveNow && (LastPacketNs != 0) && (QueueCount == 0U))
        {
            NowNs = GetMonotonicTimeNs();
            if(((NowNs - LastPacketNs) / 1000ULL) >= VSPKSTALLDETECTUS)
            {
                if(!GapMuteActive)
                    P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrStallEvents, 1U);
                PrefillActive = true;
                GapMuteActive = true;
            }
        }

        if(QueueCount == 0)
        {
            if(!GapMuteActive)
                continue;

            Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
            if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
            if(FIFOUnderflow)
                PrefillActive = true;
            NoteSpeakerUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive,
                                 Current, QueueCount, 0, PrefillActive, GapMuteActive);

            SilenceFramesToWrite = GetSpeakerGapSilenceFrames(Current, Depth / VMEMWORDSPERFRAME);
            if(SilenceFramesToWrite == 0)
                continue;

            WriteBytes = SilenceFramesToWrite * VDMATRANSFERSIZE;
            memset(SpkBasePtr, 0, WriteBytes);
            if(DMAWriteToFPGA(DMAWritefile_fd, SpkBasePtr, WriteBytes, VADDRSPKRSTREAMWRITE) < 0)
            {
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAErrors, 1U);
                atomic_store(&ThreadError, true);
                break;
            }
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWrites, 1U);
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWriteBytes, WriteBytes);
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrSilenceFrames, SilenceFramesToWrite);
            continue;
        }

        QueueAgeUs = 0;
        if(QueueArrivalNs[QueueHead] != 0)
            QueueAgeUs = (GetMonotonicTimeNs() - QueueArrivalNs[QueueHead]) / 1000ULL;

        Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);        // refresh actual FIFO occupancy before deciding to keep batching
        if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
            printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
        if(FIFOUnderflow)
            PrefillActive = true;
        NoteSpeakerUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive,
                             Current, QueueCount, QueueAgeUs, PrefillActive, GapMuteActive);

        FramesToWrite = GetSpeakerWriteFrames(Current, &PrefillActive, QueueCount, QueueAgeUs);
        if(FramesToWrite == 0)
            continue;

        while (((Depth / VMEMWORDSPERFRAME) == 0U) && !atomic_load(&ExitRequested))
        {
            usleep(500);
            Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
            if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
            if(FIFOUnderflow)
                PrefillActive = true;
            NoteSpeakerUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive,
                                 Current, QueueCount, QueueAgeUs, PrefillActive, GapMuteActive);
        }
        if(atomic_load(&ExitRequested))
            break;

        if(FramesToWrite > (Depth / VMEMWORDSPERFRAME))
            FramesToWrite = Depth / VMEMWORDSPERFRAME;
        if(FramesToWrite == 0)
            continue;

        for(FrameIndex = 0; FrameIndex < FramesToWrite; FrameIndex++)
        {
            memcpy(SpkBasePtr + (FrameIndex * VDMATRANSFERSIZE), QueueBuffers[QueueHead], VDMATRANSFERSIZE);
            QueueArrivalNs[QueueHead] = 0;
            QueueHead = (QueueHead + 1U) % VSPKSOFTQUEUEFRAMES;
            QueueCount--;
        }

        WriteBytes = FramesToWrite * VDMATRANSFERSIZE;
        if(DMAWriteToFPGA(DMAWritefile_fd, SpkBasePtr, WriteBytes, VADDRSPKRSTREAMWRITE) < 0)
        {
            P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAErrors, 1U);
            atomic_store(&ThreadError, true);
            break;
        }
        P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWrites, 1U);
        P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWriteBytes, WriteBytes);
        Current += FramesToWrite * VMEMWORDSPERFRAME;
        if(Current >= (VSPKPREFILLHIGHFRAMES * VMEMWORDSPERFRAME))
            PrefillActive = false;
    }
//
// close down thread
//
cleanup:
    if(DMAWritefile_fd >= 0)
        close(DMAWritefile_fd);
    free(SpkWriteBuffer);
    if(atomic_load(&ThreadData->Socketid) > 0)
        close(atomic_load(&ThreadData->Socketid));    // close incoming data socket
    atomic_store(&ThreadData->Socketid, 0);
    atomic_store(&ThreadData->Active, false);     // indicate it is closed
    return NULL;
}
