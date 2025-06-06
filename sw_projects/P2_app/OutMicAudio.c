/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// OutMicAudio.c:
//
// handle "outgoing microphone audio" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "OutMicAudio.h"
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
#include <time.h>

#define VMICSAMPLESPERFRAME 64
#define VDMABUFFERSIZE 32768
#define VALIGNMENT 4096
#define VBASE 0x1000
#define VDMATRANSFERSIZE 128
#define VSTARTUPDELAY 100
#define MAX_AUDIO_LEVEL 30000

// Move DMAReadfile_fd to a shared source or header as extern if needed across files
int DMAReadfile_fd = -1;

void *OutgoingMicSamples(void *arg)
{
    struct iovec iovecinst;
    struct msghdr datagram;
    uint8_t UDPBuffer[VMICPACKETSIZE];
    uint32_t SequenceCounter = 0;

    struct ThreadSocketData* ThreadData;
    struct sockaddr_in DestAddr;
    bool InitError = false;
    int Error;

    uint8_t* MicReadBuffer = NULL;
    uint32_t MicBufferSize = VDMABUFFERSIZE;
    unsigned char* MicBasePtr;
    uint32_t Depth = 0;
    uint32_t RegisterValue;
    bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;
    unsigned int Current;
    unsigned int StartupCount;

    const int sndbuf = 1024 * 1024;
    struct timespec lastSendTime;

    ThreadData = (struct ThreadSocketData *)arg;
    ThreadData->Active = true;
    printf("spinning up outgoing mic thread with port %d, pid=%ld\n", ThreadData->Portid, syscall(SYS_gettid));

    fcntl(ThreadData->Socketid, F_SETFL, fcntl(ThreadData->Socketid, F_GETFL, 0) | O_NONBLOCK);
    setsockopt(ThreadData->Socketid, SOL_SOCKET, SO_SNDBUF, &sndbuf, sizeof(sndbuf));
    clock_gettime(CLOCK_MONOTONIC, &lastSendTime);

    if (posix_memalign((void**)&MicReadBuffer, VALIGNMENT, MicBufferSize) != 0 || !MicReadBuffer) {
        printf("mic read buffer allocation failed\n");
        InitError = true;
    }
    MicBasePtr = MicReadBuffer + VBASE;
    if (MicReadBuffer)
        memset(MicReadBuffer, 0, MicBufferSize);

    if (DMAReadfile_fd < 0) {
        DMAReadfile_fd = open(VMICDMADEVICE, O_RDWR);
        if (DMAReadfile_fd < 0) {
            printf("XDMA read device open failed for mic data\n");
            InitError = true;
        }
    }

    SetupFIFOMonitorChannel(eMicCodecDMA, false);
    ResetDMAStreamFIFO(eMicCodecDMA);
    RegisterValue = ReadFIFOMonitorChannel(eMicCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
    if(UseDebug)
        printf("mic FIFO Depth register = %08x (should be ~0)\n", RegisterValue);

    while (!InitError) {
        while(!SDRActive) {
            if(ThreadData->Cmdid & VBITCHANGEPORT) {
                printf("Mic data request change port\n");
                close(ThreadData->Socketid);
                MakeSocket(ThreadData, 0);
                fcntl(ThreadData->Socketid, F_SETFL, fcntl(ThreadData->Socketid, F_GETFL, 0) | O_NONBLOCK);
                setsockopt(ThreadData->Socketid, SOL_SOCKET, SO_SNDBUF, &sndbuf, sizeof(sndbuf));
                ThreadData->Cmdid &= ~VBITCHANGEPORT;
            }
            usleep(100);
        }

        printf("starting activity on mic thread\n");
        StartupCount = VSTARTUPDELAY;
        SequenceCounter = 0;
        memcpy(&DestAddr, &reply_addr, sizeof(struct sockaddr_in));
        memset(&iovecinst, 0, sizeof(struct iovec));
        memset(&datagram, 0, sizeof(datagram));
        iovecinst.iov_base = UDPBuffer;
        iovecinst.iov_len = VMICPACKETSIZE;
        datagram.msg_iov = &iovecinst;
        datagram.msg_iovlen = 1;
        datagram.msg_name = &DestAddr;
        datagram.msg_namelen = sizeof(DestAddr);

        while(SDRActive && !InitError) {
            Depth = ReadFIFOMonitorChannel(eMicCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
            if((StartupCount == 0) && FIFOOverThreshold) {
                GlobalFIFOOverflows |= 0b00000010;
                if(UseDebug)
                    printf("Codec Mic FIFO Overthreshold, depth now = %d\n", Current);
            }
            if((StartupCount == 0) && FIFOUnderflow) {
                if(UseDebug)
                    printf("Codec Mic FIFO Underflowed, depth now = %d\n", Current);
            }

            while (Depth < (VMICSAMPLESPERFRAME/4)) {
                struct timespec ts = {0, 1000000};
                nanosleep(&ts, NULL);
                Depth = ReadFIFOMonitorChannel(eMicCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
                if((StartupCount == 0) && FIFOOverThreshold) {
                    GlobalFIFOOverflows |= 0b00000010;
                    if(UseDebug)
                        printf("Codec Mic FIFO Overthreshold, depth now = %d\n", Current);
                }
                if((StartupCount == 0) && FIFOUnderflow) {
                    if(UseDebug)
                        printf("Codec Mic FIFO Underflowed, depth now = %d\n", Current);
                }
            }

            sem_wait(&MicWBDMAMutex);
            DMAReadFromFPGA(DMAReadfile_fd, MicBasePtr, VDMATRANSFERSIZE, VADDRMICSTREAMREAD);
            sem_post(&MicWBDMAMutex);

            int16_t* audioData = (int16_t*)MicBasePtr;
            int16_t maxValue = 0;
            for (int i = 0; i < VMICSAMPLESPERFRAME; i++) {
                int16_t sample = audioData[i];
                if (abs(sample) > maxValue) {
                    maxValue = abs(sample);
                }
            }
            if (maxValue > MAX_AUDIO_LEVEL) {
                float scale = (float)MAX_AUDIO_LEVEL / maxValue;
                for (int i = 0; i < VMICSAMPLESPERFRAME; i++) {
                    audioData[i] = (int16_t)(audioData[i] * scale);
                }
            }

            *(uint32_t*)UDPBuffer = htonl(SequenceCounter++);
            memcpy(UDPBuffer+4, MicBasePtr, VDMATRANSFERSIZE);
            Error = sendmsg(ThreadData->Socketid, &datagram, 0);
            if (Error == -1) {
                if (errno == EAGAIN) {
                    printf("sendmsg: Socket send buffer full, packet dropped\n");
                } else {
                    perror("sendmsg, Mic Audio");
                    InitError = true;
                }
            } else {
                struct timespec currentTime;
                clock_gettime(CLOCK_MONOTONIC, &currentTime);
                double sendInterval = (currentTime.tv_sec - lastSendTime.tv_sec) + (currentTime.tv_nsec - lastSendTime.tv_nsec) / 1e9;
                lastSendTime = currentTime;
                if (UseDebug) printf("Send interval: %f seconds\n", sendInterval);
            }
            if(StartupCount != 0)
                StartupCount--;
        }
    }

    if (MicReadBuffer)
        free(MicReadBuffer);

    if(InitError)
      ThreadError = true;

    printf("shutting down outgoing mic data thread\n");
    close(ThreadData->Socketid); 
    ThreadData->Active = false;
    return NULL;
}
