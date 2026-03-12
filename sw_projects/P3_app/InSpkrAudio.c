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
#include <syscall.h>
#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"


#define VSPKSAMPLESPERFRAME 64                      // samples per UDP frame
#define VMEMWORDSPERFRAME 32                        // 8 byte writes per UDP msg
#define VSPKSAMPLESPERMEMWORD 2                     // 2 samples (each 4 bytres) per 8 byte word
#define VDMABUFFERSIZE 32768						// memory buffer to reserve
#define VALIGNMENT 4096                             // buffer alignment
#define VBASE 0x1000								// DMA start at 4K into buffer
#define VDMATRANSFERSIZE 256                        // write 1 message at a time
#define VMAXDMABATCHFRAMES 8                        // opportunistically batch queued UDP frames into one DMA
#define VSTARTUPDELAY 100                           // 100 messages (~100ms) before reporting under or overflows


//
// listener thread for incoming DDC (speaker) audio packets
// planned strategy: just DMA spkr data when available; don't copy and DMA a larger amount.
// if sufficient FIFO data available: DMA that data and transfer it out. 
// if it turns out to be too inefficient, we'll have to try larger DMA.
//
void *IncomingSpkrAudio(void *arg)                      // listener thread
{
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    struct sockaddr_in addr_from;                         // holds MAC address of source of incoming messages
    uint8_t UDPInBuffer[VSPEAKERAUDIOSIZE];               // incoming buffer
    struct iovec iovecinst;                               // iovcnt buffer - 1 for each outgoing buffer
    struct msghdr datagram;                               // multiple incoming message header
    int size;                                             // UDP datagram length

//
// variables for DMA buffer 
//
    uint8_t* SpkWriteBuffer = NULL;							// data for DMA to write to spkr
    uint32_t SpkBufferSize = VDMABUFFERSIZE;
    unsigned char* SpkBasePtr;								// ptr to DMA location in spk memory
    uint32_t Depth = 0;
    int DMAWritefile_fd = -1;								// DMA read file device
    bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;
    uint32_t RegVal;
    unsigned int Current;                                   // current occupied locations in FIFO
    unsigned int StartupCount = 0;                          // used to delay reporting of under & overflows
    bool PrevSDRActive = false;                             // used to detect change of state
    uint32_t BatchFrames = 0;
    uint32_t BatchBytes = 0;


    ThreadData = (struct ThreadSocketData *)arg;
    atomic_store(&ThreadData->Active, true);
    printf("spinning up speaker audio thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));

    //
    // setup DMA buffer
    //
    posix_memalign((void**)&SpkWriteBuffer, VALIGNMENT, SpkBufferSize);
    if (!SpkWriteBuffer)
        printf("spkr write buffer allocation failed\n");
    SpkBasePtr = SpkWriteBuffer + VBASE;
    memset(SpkWriteBuffer, 0, SpkBufferSize);

    //
    // open DMA device driver
    // opened write only to accommodate potential use of a different XDMA device driver
    //
    DMAWritefile_fd = open(VSPKDMADEVICE, O_WRONLY);
    if (DMAWritefile_fd < 0)
        printf("XDMA write device open failed for spk data\n");
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
            StartupCount = VSTARTUPDELAY;
        PrevSDRActive = SDRActiveNow;

        memset(&iovecinst, 0, sizeof(struct iovec));            // clear buffers
        memset(&datagram, 0, sizeof(datagram));
        iovecinst.iov_base = &UDPInBuffer;                      // set buffer for incoming message number i
        iovecinst.iov_len = VSPEAKERAUDIOSIZE;
        datagram.msg_iov = &iovecinst;
        datagram.msg_iovlen = 1;
        datagram.msg_name = &addr_from;
        datagram.msg_namelen = sizeof(addr_from);
        //
        // receive operation thread
        //
        size = recvmsg(atomic_load(&ThreadData->Socketid), &datagram, 0);   // get one message. If it times out, sets size=-1
        if(size < 0 && errno != EAGAIN)
        {
            perror("recvfrom fail, Speaker data");
            atomic_store(&ThreadError, true);
            break;
        }
        if(size == VSPEAKERAUDIOSIZE)                           // we have received a packet!
        {
            BatchFrames = 0;
            BatchBytes = 0;
            while((size == VSPEAKERAUDIOSIZE) && (BatchFrames < VMAXDMABATCHFRAMES))
            {
                if(StartupCount != 0)                                   // decrement startup message count
                    StartupCount--;
                atomic_store(&NewMessageReceived, true);
                RegVal += 1;            //debug
                memcpy(SpkBasePtr + BatchBytes, UDPInBuffer + 4, VDMATRANSFERSIZE);              // copy out spk samples
                BatchBytes += VDMATRANSFERSIZE;
                BatchFrames++;
                size = recvmsg(atomic_load(&ThreadData->Socketid), &datagram, MSG_DONTWAIT);
                if(size < 0)
                {
                    if((errno == EAGAIN) || (errno == EWOULDBLOCK))
                        break;
                    perror("recvfrom fail while draining Speaker data");
                    atomic_store(&ThreadError, true);
                    break;
                }
            }
            if(atomic_load(&ThreadError))
                break;
            Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);        // read the FIFO free locations
            if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
            if((StartupCount == 0) && FIFOUnderflow)
            {
                pthread_mutex_lock(&g_fifo_overflow_mutex);
                GlobalFIFOOverflows |= 0b00001000;
                pthread_mutex_unlock(&g_fifo_overflow_mutex);
                if(UseDebug)
                    printf("Codec speaker FIFO Underflowed, depth now = %d\n", Current);
            }

            while ((Depth < (VMEMWORDSPERFRAME * BatchFrames)) && !atomic_load(&ExitRequested))
            {
                usleep(1000);
                Depth = ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
                if((StartupCount == 0) && FIFOOverThreshold && UseDebug)
                    printf("Codec speaker FIFO Overthreshold, depth now = %d\n", Current);
                if((StartupCount == 0) && FIFOUnderflow)
                {
                    pthread_mutex_lock(&g_fifo_overflow_mutex);
                    GlobalFIFOOverflows |= 0b00001000;
                    pthread_mutex_unlock(&g_fifo_overflow_mutex);
                    if(UseDebug)
                        printf("Codec speaker FIFO Underflowed, depth now = %d\n", Current);
                }
            }
            if(atomic_load(&ExitRequested))
                break;
            if(BatchBytes != 0)
                DMAWriteToFPGA(DMAWritefile_fd, SpkBasePtr, BatchBytes, VADDRSPKRSTREAMWRITE);
        }
    }
//
// close down thread
//
    close(atomic_load(&ThreadData->Socketid));    // close incoming data socket
    atomic_store(&ThreadData->Socketid, 0);
    atomic_store(&ThreadData->Active, false);     // indicate it is closed
    return NULL;
}
