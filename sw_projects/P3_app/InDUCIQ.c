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
#include "../common/p23_perf_telemetry.h"
#include <pthread.h>
#include <syscall.h>




#define VIQSAMPLESPERFRAME 240                      // samples per UDP frame
#define VMEMWORDSPERFRAME 180                       // memory writes per UDP frame
#define VBYTESPERSAMPLE 6							// 24 bit + 24 bit samples
#define VDMABUFFERSIZE 32768						// memory buffer to reserve
#define VALIGNMENT 4096                             // buffer alignment
#define VBASE 0x1000								// DMA start at 4K into buffer
#define VDMATRANSFERSIZE 1440                       // write 1 message at a time
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

//
// listener thread for incoming DUC I/Q packets
// planned strategy: just DMA spkr data when available; don't copy and DMA a larger amount.
// if sufficient FIFO data available: DMA that data and transfer it out. 
// if it turns out to be too inefficient, we'll have to try larger DMA.
//
void *IncomingDUCIQ(void *arg)                          // listener thread
{
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    uint8_t UDPInBuffers[VMAXDMABATCHFRAMES][VDUCIQSIZE];
    struct iovec IovecList[VMAXDMABATCHFRAMES];
    struct mmsghdr DatagramList[VMAXDMABATCHFRAMES];
    unsigned int MsgIndex = 0;
    int Received = 0;

                                                          //
// variables for DMA buffer 
//
    uint8_t* IQWriteBuffer = NULL;							// data for DMA to write to DUC
    uint32_t IQBufferSize = VDMABUFFERSIZE;
    unsigned char* IQBasePtr;								// ptr to DMA location in I/Q memory
    uint32_t Depth = 0;
    int DMAWritefile_fd = -1;								// DMA read file device
    bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;
    uint32_t Cntr;                                          // sample counter
    uint8_t* SrcPtr;                                        // pointer to data from Thetis
    uint8_t* DestPtr;                                       // pointer to DMA buffer data
    unsigned int Current = 0;                               // current occupied locations in FIFO
    unsigned int StartupCount = 0;                          // used to delay reporting of under & overflows
    bool PrevSDRActive = false;                             // used to detect change of state
    bool UnderflowActive = false;                           // tracks one continuous starvation episode
    bool PrefillActive = true;                              // maintains a DUC FIFO cushion after startup or underrun
    uint32_t PendingFrames = 0;
    uint32_t PendingBytes = 0;
    uint64_t PendingStartNs = 0;
    uint32_t TargetFrames = VDUCPREFILLQUEUEFRAMES;
    uint64_t QueueAgeUs = 0;
    uint64_t MaxQueueAgeUs = VDUCMAXQUEUEAGEUS;
    struct timespec ReceiveTimeout;
    struct timespec *ReceiveTimeoutPtr = NULL;

    ThreadData = (struct ThreadSocketData *)arg;
    atomic_store(&ThreadData->Active, true);
    printf("spinning up DUC I/Q thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
    ApplyCriticalAudioThreadRuntime("DUC I/Q");

    memset(DatagramList, 0, sizeof(DatagramList));
    for (MsgIndex = 0; MsgIndex < VMAXDMABATCHFRAMES; MsgIndex++)
    {
        IovecList[MsgIndex].iov_base = UDPInBuffers[MsgIndex];
        IovecList[MsgIndex].iov_len = VDUCIQSIZE;
        DatagramList[MsgIndex].msg_hdr.msg_iov = &IovecList[MsgIndex];
        DatagramList[MsgIndex].msg_hdr.msg_iovlen = 1;
    }
  
    //
    // setup DMA buffer
    //
    if (posix_memalign((void**)&IQWriteBuffer, VALIGNMENT, IQBufferSize) != 0)
    {
        IQWriteBuffer = NULL;
        printf("I/Q TX write buffer allocation failed\n");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
    IQBasePtr = IQWriteBuffer + VBASE;
    memset(IQWriteBuffer, 0, IQBufferSize);

    //
    // open DMA device driver
    // opened write only to accommodate potential use of a different XDMA device driver
    //
    DMAWritefile_fd = open(VDUCDMADEVICE, O_WRONLY);
    if (DMAWritefile_fd < 0)
    {
        perror("XDMA write device open failed for TX I/Q data");
        atomic_store(&ThreadError, true);
        goto cleanup;
    }
        
//
// setup hardware
//
    EnableDUCMux(false);                                  // disable temporarily
    SetTXIQDeinterleaved(false);                          // not interleaved (at least for now!)
    ResetDUCMux();                                        // reset 64 to 48 mux
    ResetDMAStreamFIFO(eTXDUCDMA);
    SetupFIFOMonitorChannel(eTXDUCDMA, false);
    EnableDUCMux(true);                                   // enable operation

  //
  // main processing loop
  //
    while(!atomic_load(&ExitRequested))
    {
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
        }

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
                if(ElapsedAgeUs < VDUCMAXQUEUEAGEUS)
                    RemainingAgeUs = VDUCMAXQUEUEAGEUS - ElapsedAgeUs;
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
                perror("recvfrom fail, TX I/Q data");
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCRecvErrors, 1U);
                atomic_store(&ThreadError, true);
                break;
            }
            if(Received < 0)
                Received = 0;
        }
        else
            Received = 0;

        if(Received > 0)
        {
            for(MsgIndex = 0; MsgIndex < (unsigned int)Received; MsgIndex++)
            {
                if(DatagramList[MsgIndex].msg_len != VDUCIQSIZE)
                    continue;
                if(PendingFrames == 0)
                    PendingStartNs = GetMonotonicTimeNs();
                if(StartupCount != 0)                                   // decrement startup message count
                    StartupCount--;
                atomic_store(&NewMessageReceived, true);
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCPackets, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCBytes, VDUCIQSIZE);
                SrcPtr = (uint8_t *) (UDPInBuffers[MsgIndex] + 4);
                DestPtr = (uint8_t *) (IQBasePtr + PendingBytes);
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

        Depth = ReadFIFOMonitorChannel(eTXDUCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);           // refresh actual FIFO occupancy before deciding to keep batching
        if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
            printf("TX DUC FIFO Overthreshold, depth now = %d\n", Current);
        if(FIFOUnderflow)
            PrefillActive = true;
        NoteDUCUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);

        TargetFrames = GetDUCTargetFrames(Current, &PrefillActive);
        MaxQueueAgeUs = GetDUCQueueAgeLimitUs(Current, &TargetFrames);
        if((PendingFrames < TargetFrames) && (QueueAgeUs < MaxQueueAgeUs) && (PendingFrames < VMAXDMABATCHFRAMES))
            continue;

        {
            while ((Depth < (VMEMWORDSPERFRAME * PendingFrames)) && !atomic_load(&ExitRequested))      // loop till space available
            {
                usleep(500);								                    // 0.5ms wait
                Depth = ReadFIFOMonitorChannel(eTXDUCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);       // read the FIFO free locations
                if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                    printf("TX DUC FIFO Overthreshold, depth now = %d\n", Current);
                if(FIFOUnderflow)
                    PrefillActive = true;
                NoteDUCUnderflow((StartupCount == 0) && SDRActiveNow, FIFOUnderflow, &UnderflowActive, Current);
            }
            if(atomic_load(&ExitRequested))
                break;
            if(PendingBytes != 0)
            {
                if(DMAWriteToFPGA(DMAWritefile_fd, IQBasePtr, PendingBytes, VADDRDUCSTREAMWRITE) < 0)
                {
                    P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAErrors, 1U);
                    atomic_store(&ThreadError, true);
                    break;
                }
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAWrites, 1U);
                P23PerfTelemetryCounterAdd(eP23PerfCounterDUCDMAWriteBytes, PendingBytes);
                Current += PendingFrames * VMEMWORDSPERFRAME;
                if(Current >= (VDUCPREFILLHIGHFRAMES * VMEMWORDSPERFRAME))
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
    free(IQWriteBuffer);
    if(atomic_load(&ThreadData->Socketid) > 0)
        close(atomic_load(&ThreadData->Socketid));    // close incoming data socket
    atomic_store(&ThreadData->Socketid, 0);
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
