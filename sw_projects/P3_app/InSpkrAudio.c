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
#include "../common/p23_perf_telemetry.h"


#define VSPKSAMPLESPERFRAME 64                      // samples per UDP frame
#define VMEMWORDSPERFRAME 32                        // 8 byte writes per UDP msg
#define VSPKSAMPLESPERMEMWORD 2                     // 2 samples (each 4 bytres) per 8 byte word
#define VDMABUFFERSIZE 32768						// memory buffer to reserve
#define VALIGNMENT 4096                             // buffer alignment
#define VBASE 0x1000								// DMA start at 4K into buffer
#define VDMATRANSFERSIZE 256                        // write 1 message at a time
#define VSPKNORMALQUEUEFRAMES 3                     // normal queue depth before a DMA write
#define VSPKPREFILLQUEUEFRAMES 6                    // refill deeper after startup or underrun recovery
#define VMAXDMABATCHFRAMES 8                        // max queued frames we coalesce into one DMA write
#define VSTARTUPDELAY 100                           // 100 messages (~100ms) before reporting under or overflows
#define VSPKPREFILLLOWFRAMES 4                      // re-enter prefill when FIFO occupancy falls below this
#define VSPKPREFILLHIGHFRAMES 12                    // stay in prefill until FIFO occupancy reaches this
#define VSPKMAXQUEUEAGEUS 2500U                     // bound extra speaker latency while still allowing coalescing

static void NoteSpeakerUnderflow(bool ReportingEnabled, bool Underflowed, bool *UnderflowActive,
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
    GlobalFIFOOverflows |= 0b00001000;
    pthread_mutex_unlock(&g_fifo_overflow_mutex);

    if (!*UnderflowActive)
    {
        P23PerfTelemetryCounterAdd(eP23PerfCounterFIFOSpkrUnder, 1U);
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


//
// listener thread for incoming DDC (speaker) audio packets
// planned strategy: just DMA spkr data when available; don't copy and DMA a larger amount.
// if sufficient FIFO data available: DMA that data and transfer it out. 
// if it turns out to be too inefficient, we'll have to try larger DMA.
//
void *IncomingSpkrAudio(void *arg)                      // listener thread
{
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    uint8_t UDPInBuffers[VMAXDMABATCHFRAMES][VSPEAKERAUDIOSIZE];
    struct iovec IovecList[VMAXDMABATCHFRAMES];
    struct mmsghdr DatagramList[VMAXDMABATCHFRAMES];
    unsigned int MsgIndex = 0;
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
    uint32_t PendingFrames = 0;
    uint32_t PendingBytes = 0;
    uint64_t PendingStartNs = 0;
    uint32_t TargetFrames = VSPKPREFILLQUEUEFRAMES;
    uint64_t QueueAgeUs = 0;
    struct timespec ReceiveTimeout;
    struct timespec *ReceiveTimeoutPtr = NULL;


    ThreadData = (struct ThreadSocketData *)arg;
    atomic_store(&ThreadData->Active, true);
    printf("spinning up speaker audio thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));

    memset(DatagramList, 0, sizeof(DatagramList));
    for (MsgIndex = 0; MsgIndex < VMAXDMABATCHFRAMES; MsgIndex++)
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
            PendingFrames = 0;
            PendingBytes = 0;
            PendingStartNs = 0;
        }
        else if(!SDRActiveNow)
        {
            UnderflowActive = false;
            PrefillActive = true;
            Current = 0;
            PendingFrames = 0;
            PendingBytes = 0;
            PendingStartNs = 0;
        }
        PrevSDRActive = SDRActiveNow;

        if(PendingFrames < VMAXDMABATCHFRAMES)
        {
            unsigned int ReceiveGoal = VMAXDMABATCHFRAMES - PendingFrames;
            int ReceiveFlags = MSG_WAITFORONE;

            ReceiveTimeoutPtr = NULL;
            if((PendingFrames != 0) && (PendingStartNs != 0))
            {
                uint64_t RemainingAgeUs = 0;
                uint64_t ElapsedAgeUs = (GetMonotonicTimeNs() - PendingStartNs) / 1000ULL;
                if(ElapsedAgeUs < VSPKMAXQUEUEAGEUS)
                    RemainingAgeUs = VSPKMAXQUEUEAGEUS - ElapsedAgeUs;
                if(RemainingAgeUs == 0)
                    Received = 0;
                else
                {
                    ReceiveTimeout.tv_sec = (time_t)(RemainingAgeUs / 1000000ULL);
                    ReceiveTimeout.tv_nsec = (long)((RemainingAgeUs % 1000000ULL) * 1000ULL);
                    ReceiveTimeoutPtr = &ReceiveTimeout;
                    Received = recvmmsg(atomic_load(&ThreadData->Socketid), DatagramList, ReceiveGoal, ReceiveFlags, ReceiveTimeoutPtr);
                }
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
                if(DatagramList[MsgIndex].msg_len != VSPEAKERAUDIOSIZE)
                    continue;
                if(PendingFrames == 0)
                    PendingStartNs = GetMonotonicTimeNs();
                if(StartupCount != 0)                                   // decrement startup message count
                    StartupCount--;
                atomic_store(&NewMessageReceived, true);
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrPackets, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrBytes, VSPEAKERAUDIOSIZE);
                RegVal += 1;            //debug
                memcpy(SpkBasePtr + PendingBytes, UDPInBuffers[MsgIndex] + 4, VDMATRANSFERSIZE); // queue speaker samples for the next DMA
                PendingBytes += VDMATRANSFERSIZE;
                PendingFrames++;
                if(PendingFrames >= VMAXDMABATCHFRAMES)
                    break;
            }
        }

        if(PendingFrames == 0)
            continue;

        QueueAgeUs = 0;
        if(PendingStartNs != 0)
            QueueAgeUs = (GetMonotonicTimeNs() - PendingStartNs) / 1000ULL;

        TargetFrames = GetSpeakerTargetFrames(Current, &PrefillActive);
        if((PendingFrames < TargetFrames) && (QueueAgeUs < VSPKMAXQUEUEAGEUS) && (PendingFrames < VMAXDMABATCHFRAMES))
            continue;

        {
            Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);        // read the FIFO free locations
            if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
            if(FIFOUnderflow)
                PrefillActive = true;
            NoteSpeakerUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);

            while ((Depth < (VMEMWORDSPERFRAME * PendingFrames)) && !atomic_load(&ExitRequested))
            {
                usleep(1000);
                Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
                if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                    printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
                if(FIFOUnderflow)
                    PrefillActive = true;
                NoteSpeakerUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);
            }
            if(atomic_load(&ExitRequested))
                break;
            if(PendingBytes != 0)
            {
                if(DMAWriteToFPGA(DMAWritefile_fd, SpkBasePtr, PendingBytes, VADDRSPKRSTREAMWRITE) < 0)
                {
                    P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAErrors, 1U);
                    atomic_store(&ThreadError, true);
                    break;
                }
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWrites, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterSpkrDMAWriteBytes, PendingBytes);
                Current += PendingFrames * VMEMWORDSPERFRAME;
                if(Current >= (VSPKPREFILLHIGHFRAMES * VMEMWORDSPERFRAME))
                    PrefillActive = false;
                PendingFrames = 0;
                PendingBytes = 0;
                PendingStartNs = 0;
            }
        }
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
