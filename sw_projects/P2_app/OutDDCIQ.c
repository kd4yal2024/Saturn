/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// OutDDCIQ.c:
//
// handle "outgoing DDC I/Q data" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "OutMicAudio.h"
#include <errno.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <fcntl.h>
#include <pthread.h>
#include <syscall.h>
#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/debugaids.h"

#define VDMABUFFERSIZE 131072
#define VALIGNMENT 4096
#define VBASE 0x1000
#define VDMATRANSFERSIZE 4096
#define VDDCPACKETSIZE 1444
#define VIQSAMPLESPERFRAME 238
#define VIQBYTESPERFRAME 6*VIQSAMPLESPERFRAME
#define VSTARTUPDELAY 100

uint8_t* DMAReadBuffer = NULL;
uint32_t DMABufferSize = VDMABUFFERSIZE;
unsigned char* DMAReadPtr;
unsigned char* DMAHeadPtr;
unsigned char* DMABasePtr;

uint8_t* UDPBuffer[VNUMDDC];
uint8_t* DDCSampleBuffer[VNUMDDC];
unsigned char* IQReadPtr[VNUMDDC];
unsigned char* IQHeadPtr[VNUMDDC];
unsigned char* IQBasePtr[VNUMDDC];

bool CreateDynamicMemory(void)
{
    uint32_t DDC;
    bool Result = false;
    posix_memalign((void**)&DMAReadBuffer, VALIGNMENT, DMABufferSize);
    DMAReadPtr = DMAReadBuffer + VBASE;
    DMAHeadPtr = DMAReadBuffer + VBASE;
    DMABasePtr = DMAReadBuffer + VBASE;
    if (!DMAReadBuffer)
    {
        printf("I/Q read buffer allocation failed\n");
        Result = true;
    }
    memset(DMAReadBuffer, 0, DMABufferSize);
    for (DDC = 0; DDC < VNUMDDC; DDC++)
    {
        UDPBuffer[DDC] = malloc(VDDCPACKETSIZE);
        DDCSampleBuffer[DDC] = malloc(DMABufferSize);
        IQReadPtr[DDC] = DDCSampleBuffer[DDC] + VBASE;
        IQHeadPtr[DDC] = DDCSampleBuffer[DDC] + VBASE;
        IQBasePtr[DDC] = DDCSampleBuffer[DDC] + VBASE;
    }
    return Result;
}

void FreeDynamicMemory(void)
{
    uint32_t DDC;
    free(DMAReadBuffer);
    for (DDC = 0; DDC < VNUMDDC; DDC++)
    {
        free(UDPBuffer[DDC]);
        free(DDCSampleBuffer[DDC]);
    }
}

void *OutgoingDDCIQ(void *arg)
{
    uint32_t DMATransferSize;
    bool InitError = false;
    uint32_t ResidueBytes;
    uint32_t Depth = 0;
    int IQReadfile_fd = -1;
    uint32_t RegisterValue;
    bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;
    int DDC;
    struct ThreadSocketData *ThreadData;
    struct sockaddr_in DestAddr[VNUMDDC];
    struct iovec iovecinst[VNUMDDC];
    struct msghdr datagram[VNUMDDC];
    uint32_t SequenceCounter[VNUMDDC];
    uint32_t FrameLength;
    uint32_t DDCCounts[VNUMDDC];
    uint32_t RateWord;
    uint32_t HdrWord;
    uint16_t* SrcWordPtr, * DestWordPtr;
    uint32_t *LongWordPtr;
    uint32_t PrevRateWord;
    uint32_t Cntr;
    bool HeaderFound;
    uint32_t DecodeByteCount;
    unsigned int Current;
    unsigned int StartupCount;

    PrevRateWord = 0xFFFFFFFF;
    DMATransferSize = VDMATRANSFERSIZE;
    InitError = CreateDynamicMemory();
    IQReadfile_fd = open(VDDCDMADEVICE, O_RDWR);
    if (IQReadfile_fd < 0)
    {
        printf("XDMA read device open failed for DDC data\n");
        InitError = true;
    }
    ThreadData = (struct ThreadSocketData*)arg;
    printf("spinning up outgoing I/Q thread with port %d, pid=%ld\n", ThreadData->Portid, syscall(SYS_gettid));
    for (DDC = 0; DDC < VNUMDDC; DDC++)
    {
        SequenceCounter[DDC] = 0;
        (ThreadData + DDC)->Active = true;
    }
    SetRXDDCEnabled(false);
    usleep(1000);
    SetupFIFOMonitorChannel(eRXDDCDMA, false);
    ResetDMAStreamFIFO(eRXDDCDMA);
    RegisterValue = ReadFIFOMonitorChannel(eRXDDCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
    if(UseDebug)
        printf("DDC FIFO Depth register = %08x (should be ~0)\n", RegisterValue);
    Depth=0;

    while(!InitError)
    {
        while(!SDRActive)
        {
            for (DDC=0; DDC < VNUMDDC; DDC++)
                if((ThreadData+DDC) -> Cmdid & VBITCHANGEPORT)
                {
                    close((ThreadData+DDC) -> Socketid);
                    MakeSocket((ThreadData + DDC), 0);
                    (ThreadData + DDC) -> Cmdid &= ~VBITCHANGEPORT;
                }
            usleep(100);
        }
        printf("starting outgoing DDC data\n");
        StartupCount = VSTARTUPDELAY;
        for (DDC = 0; DDC < VNUMDDC; DDC++)
        {
            SequenceCounter[DDC] = 0;
            memcpy(&DestAddr[DDC], &reply_addr, sizeof(struct sockaddr_in));
            memset(&iovecinst[DDC], 0, sizeof(struct iovec));
            memset(&datagram[DDC], 0, sizeof(struct msghdr));
            iovecinst[DDC].iov_base = UDPBuffer[DDC];
            iovecinst[DDC].iov_len = VDDCPACKETSIZE;
            datagram[DDC].msg_iov = &iovecinst[DDC];
            datagram[DDC].msg_iovlen = 1;
            datagram[DDC].msg_name = &DestAddr[DDC];
            datagram[DDC].msg_namelen = sizeof(DestAddr);
        }
        printf("outDDCIQ: enable data transfer\n");
        SetRXDDCEnabled(true);
        HeaderFound = false;
        while(!InitError && SDRActive)
        {
            for (DDC = 0; DDC < VNUMDDC; DDC++)
            {
                while ((IQHeadPtr[DDC] - IQReadPtr[DDC]) > VIQBYTESPERFRAME)
                {
                    *(uint32_t*)UDPBuffer[DDC] = htonl(SequenceCounter[DDC]++);
                    memset(UDPBuffer[DDC] + 4, 0, 8);
                    *(uint16_t*)(UDPBuffer[DDC] + 12) = htons(24);
                    *(uint32_t*)(UDPBuffer[DDC] + 14) = htons(VIQSAMPLESPERFRAME);
                    memcpy(UDPBuffer[DDC] + 16, IQReadPtr[DDC], VIQBYTESPERFRAME);
                    IQReadPtr[DDC] += VIQBYTESPERFRAME;
                    int Error;
                    Error = sendmsg((ThreadData+DDC)->Socketid, &datagram[DDC], 0);
                    if(StartupCount != 0)
                        StartupCount--;
                    if (Error == -1)
                    {
                        printf("Send Error, DDC=%x, errno=%d, socket id = %d\n", DDC, errno, (ThreadData+DDC)->Socketid);
                        InitError = true;
                    }
                }
                ResidueBytes = IQHeadPtr[DDC] - IQReadPtr[DDC];
                if (IQReadPtr[DDC] > IQBasePtr[DDC])
                {
                    if (ResidueBytes != 0)
                    {
                        memcpy(IQBasePtr[DDC] - ResidueBytes, IQReadPtr[DDC], ResidueBytes);
                        IQReadPtr[DDC] = IQBasePtr[DDC] - ResidueBytes;
                    }
                    else
                        IQReadPtr[DDC] = IQBasePtr[DDC];
                    IQHeadPtr[DDC] = IQBasePtr[DDC];
                }
            }
            Depth = ReadFIFOMonitorChannel(eRXDDCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
            if((StartupCount == 0) && FIFOOverThreshold)
            {
                GlobalFIFOOverflows |= 0b00000001;
                if(UseDebug)
                    printf("RX DDC FIFO Overthreshold, depth now = %d\n", Current);
            }
            while(Depth < (DMATransferSize/8U))
            {
                usleep(500);
                Depth = ReadFIFOMonitorChannel(eRXDDCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &Current);
                if((StartupCount == 0) && FIFOOverThreshold)
                {
                    GlobalFIFOOverflows |= 0b00000001;
                    if(UseDebug)
                        printf("RX DDC FIFO Overthreshold, depth now = %d\n", Current);
                }
             }
            if(Depth > 4096)
                DMATransferSize = 32768;
            else if(Depth > 2048)
                DMATransferSize = 16384;
            else if(Depth > 1024)
                DMATransferSize = 8192;
            else
                DMATransferSize = 4096;
            DMAReadFromFPGA(IQReadfile_fd, DMAHeadPtr, DMATransferSize, VADDRDDCSTREAMREAD);
            DMAHeadPtr += DMATransferSize;
            if(HeaderFound == false)
                for(Cntr=16; Cntr < (uint32_t)(DMAHeadPtr - DMAReadPtr); Cntr+=8)
                {
                    if(*(DMAReadPtr + Cntr + 7) == 0x80)
                    {
                        HeaderFound = true;
                        DMAReadPtr += Cntr;
                        break;
                    }
                }
            if (HeaderFound == false)
            {
                InitError = true;
                exit(1);
            }
            DecodeByteCount = DMAHeadPtr - DMAReadPtr;
            while (DecodeByteCount >= 16)
            {
                if(*(DMAReadPtr + 7) != 0x80)
                {
                   // In function OutgoingDDCIQ
                    printf("header not found for rate word at addr %p\n", (void*)DMAReadPtr);
                    exit(1);
                }
                else
                {
                    LongWordPtr = (uint32_t*)DMAReadPtr;
                    RateWord = *LongWordPtr;
                    if (RateWord != PrevRateWord)
                    {
                        FrameLength = AnalyseDDCHeader(RateWord, &DDCCounts[0]);
                        PrevRateWord = RateWord;
                    }
                    if (DecodeByteCount >= ((FrameLength+1) * 8))
                    {
                        DMAReadPtr += 8;
                        SrcWordPtr = (uint16_t*)DMAReadPtr;
                        for (DDC = 0; DDC < VNUMDDC; DDC++)
                        {
                            HdrWord = DDCCounts[DDC];
                            if (HdrWord != 0)
                            {
                                DestWordPtr = (uint16_t *)IQHeadPtr[DDC];
                                for (Cntr = 0; Cntr < HdrWord; Cntr++)
                                {
                                    *DestWordPtr++ = *SrcWordPtr++;
                                    *DestWordPtr++ = *SrcWordPtr++;
                                    *DestWordPtr++ = *SrcWordPtr++;
                                    SrcWordPtr++;
                                }
                                IQHeadPtr[DDC] += 6 * HdrWord;
                            }
                        }
                        DMAReadPtr += FrameLength * 8;
                        DecodeByteCount -= (FrameLength+1) * 8;
                    }
                    else
                        break;
                }
            }
            ResidueBytes = DMAHeadPtr - DMAReadPtr;
            if (DMAReadPtr > DMABasePtr)
            {
                if (ResidueBytes != 0)
                {
                    memcpy(DMABasePtr - ResidueBytes, DMAReadPtr, ResidueBytes);
                    DMAReadPtr = DMABasePtr - ResidueBytes;
                }
                else
                    DMAReadPtr = DMABasePtr;
                DMAHeadPtr = DMABasePtr;
            }
        }
    }
    printf("shutting down DDC outgoing thread\n");
    close(ThreadData->Socketid);
    ThreadData->Active = false;
    FreeDynamicMemory();
    return NULL;
}

void HandlerCheckDDCSettings(void)
{
}