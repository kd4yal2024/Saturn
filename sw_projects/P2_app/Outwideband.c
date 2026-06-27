/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
// derived from Pavel Demin code 
//
// OutWideband.h:
//
// header: handle "outgoing wideband data" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "OutHighPriority.h"
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
#include "../common/hwaccess.h"
#include "../common/debugaids.h"
#include "../common/p23_perf_telemetry.h"


//
// global holding the current step of C&C data. Each new USB frame updates this.
//
#define VDMABUFFERSIZE 65536						            // memory buffer to reserve (2x wideband FIFO size)
#define VALIGNMENT 4096                             // buffer alignment

#define VWBPACKETSIZE 1500                          // packet size is a variable, sp make max for UDP
#define VWBSAMPLESPERFRAME 512                      // total wideband ADC samples in one WB packet
#define VWBBYTESPERFRAME 2*VWBSAMPLESPERFRAME       // total bytes in one outgoing frame
#define VWBPACKETOVERHEADBYTES 4                    // sequence counter
#define VWBMAXPAYLOADBYTES (VWBPACKETSIZE - VWBPACKETOVERHEADBYTES)
#define VWBFRAMEOFFSETBYTES 32                      // skip FPGA frame metadata before sample payload
#define VSTARTUPDELAY 100                           // 100 messages (~100ms) before reporting under or overflows
#define VNUMWBADC 2                                 // number of ADC that WB data can be collected for


//
// define the memory buffers:
//
uint8_t* WBDMAReadBuffer = NULL;								// data for DMA read from DDC
uint32_t WBDMABufferSize = VDMABUFFERSIZE;

uint8_t* WBUDPBuffer[VNUMWBADC];                                // per-ADC frame buffer
extern atomic_int DMAReadfile_fd;                            // DMA read file device (opened by mic samples thread)

//
// copies of params provided by P2 protocol
// WBParamsChanged set true if parameters have moved
//
bool WBParamsChanged;
uint8_t StoredEnables;                                          // enable bits for ADC1 (bit0) & 2 (bit1)
uint16_t StoredSamplePerPktCount;                               // samples per packet count
uint8_t StoredSampleSize;                                       // sample resolution in bits (typ 16)
uint8_t StoredRate;                                             // update rate in ms
uint8_t StoredPacketCount;                                      // packets to be transferred out
static pthread_mutex_t g_wideband_params_mutex = PTHREAD_MUTEX_INITIALIZER;


static bool WidebandParamsFitBuffers(uint16_t SampleCount, uint8_t PacketCount)
{
    uint32_t SampleBytes;
    uint32_t TotalPayloadBytes;
    uint32_t RequiredDMABytes;

    if((SampleCount == 0) || (PacketCount == 0))
        return false;

    SampleBytes = (uint32_t)SampleCount * 2U;
    if(SampleBytes > VWBMAXPAYLOADBYTES)
        return false;

    TotalPayloadBytes = (uint32_t)PacketCount * SampleBytes;
    RequiredDMABytes = VWBFRAMEOFFSETBYTES + TotalPayloadBytes;
    if(RequiredDMABytes > VDMABUFFERSIZE)
        return false;

    return true;
}



//
// create dynamically allocated memory at startup
//
bool CreateWBDynamicMemory(void)                              // return true if error
{
    uint32_t ADC;
    bool Result = false;
//
// first create the buffer for DMA, and initialise its pointers
//
    if (posix_memalign((void**)&WBDMAReadBuffer, VALIGNMENT, WBDMABufferSize) != 0)
        WBDMAReadBuffer = NULL;
    if (!WBDMAReadBuffer)
    {
        printf("Wideband read buffer allocation failed\n");
        Result = true;
    }
    else
        memset(WBDMAReadBuffer, 0, WBDMABufferSize);

    //
    // set up per-Wideband ADC data structures
    //
    for (ADC = 0; ADC < VNUMWBADC; ADC++)
    {
        WBUDPBuffer[ADC] = malloc(VWBPACKETSIZE);
        if(WBUDPBuffer[ADC] == NULL)
        {
            printf("Wideband UDP buffer allocation failed for ADC %u\n", ADC);
            Result = true;
        }
    }
    return Result;
}


void FreeWBDynamicMemory(void)
{
    uint32_t ADC;

    free(WBDMAReadBuffer);
    //
    // free the per-DDC buffers
    //
    for (ADC = 0; ADC < VNUMWBADC; ADC++)
    {
        free(WBUDPBuffer[ADC]);
        WBUDPBuffer[ADC] = NULL;
    }
}



//
// set parameters from SDR for wideband data collect
// paramters as transferred in general packet to SDR
// see if any differences are present, then store for when thread is ready
//
void SetWidebandParams(uint8_t Enables, uint16_t SampleCount, uint8_t SampleSize, uint8_t Rate, uint8_t PacketCount)
{
    bool ParamsChanged;
    uint8_t SanitizedEnables = Enables & 0x03;

    if((SanitizedEnables != 0) && !WidebandParamsFitBuffers(SampleCount, PacketCount))
    {
        printf("Invalid WB data ignored: Enables=%u, Sample/pkt=%u, Samplesize=%u, Rate=%u, PktCount=%u\n",
               (unsigned int)Enables, (unsigned int)SampleCount, (unsigned int)SampleSize,
               (unsigned int)Rate, (unsigned int)PacketCount);
        SanitizedEnables = 0;
        SampleCount = 0;
        PacketCount = 0;
    }

    pthread_mutex_lock(&g_wideband_params_mutex);
    if((SanitizedEnables != StoredEnables) || (SampleCount != StoredSamplePerPktCount) || (SampleSize != StoredSampleSize)
       || (Rate != StoredRate) || (PacketCount != StoredPacketCount))
        WBParamsChanged = true;

    StoredEnables = SanitizedEnables;                   // enable bits for ADC1 (bit0) & 2 (bit1)
    StoredSamplePerPktCount = SampleCount;                    // samples per packet count
    StoredSampleSize = SampleSize;                      // sample resolution in bits (typ 16)
    StoredRate = Rate;                                  // update rate in ms
    StoredPacketCount = PacketCount;                    // packets to be transferred out
    P23PerfTelemetrySetWidebandConfig(StoredEnables, StoredSamplePerPktCount, StoredSampleSize, StoredRate, StoredPacketCount);
    ParamsChanged = WBParamsChanged;
    pthread_mutex_unlock(&g_wideband_params_mutex);

    if(ParamsChanged)
        printf("New WB data: Enables=%d, Sample/pkt = %d, Samplesize=%d, Rate=%d, PktCount=%d\n", SanitizedEnables, SampleCount, SampleSize, Rate, PacketCount);
}


//
// read out the Wideband FIFO
// returns the number of samples read
// read available word count, then do DMA to memory buffer
//
uint32_t ReadFIFOContent()
{
    uint32_t SampleCount = 0;
    uint32_t WordCount = 0;                             // count of 64 bit words in the FIFO
    bool ADC1, ADC2;

    WordCount = GetWidebandStatus(&ADC1, &ADC2);
    if(WordCount != 0)
    {
        int LocalDMAReadFD = atomic_load(&DMAReadfile_fd);
        if(LocalDMAReadFD < 0)
            return 0;
        sem_wait(&MicWBDMAMutex);                       // get protected access
        if(DMAReadFromFPGA(LocalDMAReadFD, WBDMAReadBuffer, WordCount * 8, VADDRWIDEBANDREAD) < 0)
        {
            sem_post(&MicWBDMAMutex);                       // release protected access
            atomic_store(&ThreadError, true);
            return 0;
        }
        sem_post(&MicWBDMAMutex);                       // get protected access
        P23PerfTelemetryCounterAdd(eP23PerfCounterWidebandDMAReads, 1U);
        P23PerfTelemetryCounterAdd(eP23PerfCounterWidebandDMAReadBytes, (uint64_t)WordCount * 8U);
        SampleCount = WordCount * 4;
//        printf("word count in readFIFOContent = %d\n", WordCount);
    }
    return SampleCount;
}


//
// strategy:
// 1. We have one DMA buffer, big enough for the largest DMA from the wideband FIFO
// 2. On startup: turn off the IP and clear the FIFO if any data in it. 
// 3. when the wideband settings change: stop operation; clear FIFO; setup new settings & restart if still enabled
// 4. wideband IP started; it periodically writes defined sample count to FIFO
// 5. When write complete, a status flag is set; one for each ADC
// 6. when a flag is set, DMA out the data for that ADC then write the bit to say "data transferred"
// 7. break data into N outgoing packets and send to Thetis over UDP
// 8. Need to check if both ADCs are enabled, because more data will follow if so
// 9. when exiting: turn off the IP.
//


//
// this runs as its own thread to send outgoing wideband data
// thread initiated after a "Start" command
// will be instructed to stop & exit by main loop setting enable_thread to 0
// this code signals thread terminated by setting active_thread = 0
// substantially similar to outgoing DDC thread
//
void *OutgoingWidebandSamples(void *arg)
{
//
// memory buffers
//
    bool InitError = false;                                     // becomes true if we get an initialisation error
    
    int ADC;                                                    // iterator
    int Socketfd;
    bool LocalParamsChanged;
    uint8_t LocalEnables;
    uint16_t LocalSamplePerPktCount;
    uint8_t LocalRate;
    uint8_t LocalPacketCount;
    uint32_t SampleWordCount;                                   // no of 64 bit words required
    bool ADC1, ADC2;                                            // true if data available
    uint32_t PacketCounter;
    uint32_t StartAddress;                                      // data locations in wideband collected data
    struct ThreadSocketData *ThreadData;                        // socket etc data for each thread.
                                                                // points to 1st one
//
// variables for outgoing UDP frame
//
    struct sockaddr_in DestAddr[VNUMWBADC];                     // destination address for outgoing data
    struct iovec iovecinst[VNUMWBADC];                          // instance of iovec
    struct msghdr datagram[VNUMWBADC];
    uint32_t SequenceCounter[VNUMWBADC];                        // UDP sequence count
    

//
// initialise. Create memory buffers and open DMA file devices
// (strategy step 1)
//
    InitError = CreateWBDynamicMemory();
    //
    // note we re-use the DMA device for MIC samples
    //

    ThreadData = (struct ThreadSocketData*)arg;
    printf("spinning up outgoing Wideband sample thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));

    //
    // set up per-ADC data structures
    //
    for (ADC = 0; ADC < VNUMWBADC; ADC++)
    {
        SequenceCounter[ADC] = 0;                           // clear UDP packet counter
        atomic_store(&(ThreadData + ADC)->Active, true);    // set outgoing socket active
    }

    if(InitError)
        goto cleanup;



//
// now initialise Saturn wideband hardware.
// turn off wideband capture, and clear FIFO
// (strategy step 2)
// 
    SetWidebandEnable(false, false, false);                 // turn off data collection
    usleep(150);                                            // wait dfor any current write to end
    ReadFIFOContent();                                      // then empty the FIFO

//
// thread loop. runs continuously until commanded by main loop to exit
// initialise thread data structures;
// then while there is wideband data, make outgoing packets;
//
    while(!InitError && !atomic_load(&ExitRequested))
    {
        while(!atomic_load(&SDRActive) && !atomic_load(&ExitRequested))
        {
            // Port rebinding is handled centrally by the p2app control plane.
            usleep(100);
        }
        printf("starting outgoing Wideband data\n");
        //
        // initialise outgoing WB packet buffers - 1 per ADC
        //
        for (ADC = 0; ADC < VNUMWBADC; ADC++)
        {
            SequenceCounter[ADC] = 0;
            pthread_mutex_lock(&g_reply_addr_mutex);
            memcpy(&DestAddr[ADC], &reply_addr, sizeof(struct sockaddr_in));           // local copy of PC destination address (reply_addr is global)
            pthread_mutex_unlock(&g_reply_addr_mutex);
            memset(&iovecinst[ADC], 0, sizeof(struct iovec));
            memset(&datagram[ADC], 0, sizeof(struct msghdr));
            iovecinst[ADC].iov_base = WBUDPBuffer[ADC];
            iovecinst[ADC].iov_len = VWBPACKETSIZE;
            datagram[ADC].msg_iov = &iovecinst[ADC];
            datagram[ADC].msg_iovlen = 1;
            datagram[ADC].msg_name = &DestAddr[ADC];                   // MAC addr & port to send to
            datagram[ADC].msg_namelen = sizeof(struct sockaddr_in);
        }
      //
      // enable Saturn WB IP to transfer data
      // this is the main app loop
      // monitor changes to paramters, because this is the trigger to reconfigure operation
      //
        printf("outDDCIQ: enable data transfer\n");
        while(!InitError && atomic_load(&SDRActive) && !atomic_load(&ExitRequested))
        {
            pthread_mutex_lock(&g_wideband_params_mutex);
            LocalParamsChanged = WBParamsChanged;
            if(LocalParamsChanged)
                WBParamsChanged = false;
            LocalEnables = StoredEnables;
            LocalSamplePerPktCount = StoredSamplePerPktCount;
            LocalRate = StoredRate;
            LocalPacketCount = StoredPacketCount;
            pthread_mutex_unlock(&g_wideband_params_mutex);
//
// if parameters have changed, halt then re-load configuration (strategy step 3)
// (this will also work from a cold start)
//
            if(LocalParamsChanged)
            {
                SetWidebandEnable(false, false, false);                 // turn off data collection
                usleep(150);                                            // wait for any current write to end
                ReadFIFOContent();                                      // then empty the FIFO discarding data
                SampleWordCount = ((LocalSamplePerPktCount * LocalPacketCount) / 4) + 8;    // no. 64 bit words; over-read by 8 words
                SetWidebandSampleCount(SampleWordCount);
                SetWidebandUpdateRate(LocalRate);
                SetWidebandEnable((bool)(LocalEnables&1), (bool)(LocalEnables&2), false);
                printf("Setting WB IP: WordCount = %u, Rate = %u, ADC1 = %u, ADC2=%u\n",
                       (unsigned int)SampleWordCount, (unsigned int)LocalRate,
                       (unsigned int)(LocalEnables & 1), (unsigned int)(LocalEnables & 2));
            }
//
// then if enabled:
// using a while loop, wait for data to be available from the FPGA. 
// When it is, read it and clear the IP "data available" flag
// (strategy step 6)
// then send out packets to SDR client
// recheck if parameters have changed after a successful ready
//
            if(LocalEnables != 0)                       // if active
            {
                GetWidebandStatus(&ADC1, &ADC2);      // get flags for data available
                if(ADC1 || ADC2)                                        // if data available for either
                {
                    SampleWordCount = ReadFIFOContent();                // then read FIFO till empty
//                    printf("WB data available, ADC sample count = %d\n", SampleWordCount);
                    SetWidebandEnable((bool)(LocalEnables&1), (bool)(LocalEnables&2), true);  // re-enable record
                    //
                    // now transfer data out on UDP packets
                    // first select the buffer set yo use based on what data is available
                    //
                    if(ADC2)
                        ADC=1;
                    else
                        ADC=0;
                    SequenceCounter[ADC] = 0;                           // restart at 0 for each frame
                    for(PacketCounter = 0; PacketCounter < LocalPacketCount; PacketCounter++)
                    {
                        int Error;

                        *(uint32_t*)WBUDPBuffer[ADC] = htonl(SequenceCounter[ADC]++);     // add sequence count
                        //
                        // now add I/Q data & send outgoing packet
                        //
                        StartAddress = (PacketCounter * LocalSamplePerPktCount * 2) + 32;   // byte address; inset 4 words into recording
                        memcpy(WBUDPBuffer[ADC] + 4, WBDMAReadBuffer + StartAddress, LocalSamplePerPktCount * 2);
                        iovecinst[ADC].iov_len = LocalSamplePerPktCount * 2 + 4;           // P2 data dependent

                        Socketfd = GetThreadSocketFD(ThreadData + ADC);
                        Error = sendmsg(Socketfd, &datagram[ADC], 0);
                        if(Error != (int)iovecinst[ADC].iov_len)
                        {
                            if(Error == -1)
                                perror("sendmsg, Wideband");
                            else
                                printf("short sendmsg, Wideband: sent %d of %zu bytes\n", Error, iovecinst[ADC].iov_len);
                            P23PerfTelemetryCounterAdd(eP23PerfCounterWidebandSendErrors, 1U);
                            InitError = true;
                            break;
                        }
                        P23PerfTelemetryCounterAdd(eP23PerfCounterWidebandPackets, 1U);
                        P23PerfTelemetryCounterAdd(eP23PerfCounterWidebandBytes, (uint64_t)iovecinst[ADC].iov_len);
                        usleep(200);                    // gap between outgoing messages
                    }
                    if(InitError)
                        break;
                }
            }
            usleep(5000);

        }     // end of while(!InitError&& SDRActive) loop - typically when comm with SDR client stops
        pthread_mutex_lock(&g_wideband_params_mutex);
        StoredEnables = false;                                          // force a re-config if comm continues later
        WBParamsChanged = true;
        pthread_mutex_unlock(&g_wideband_params_mutex);
    } //end of while(!InitError)

//
// tidy shutdown of the thread
// halt the wideband IP (strategy step 9)
//
cleanup:
    if(InitError)
        atomic_store(&ThreadError, true);
    printf("shutting down Wideband outgoing thread\n");
    SetWidebandEnable(false, false, false);
    for (ADC = 0; ADC < VNUMWBADC; ADC++)
    {
        CloseThreadSocketIfOwned(ThreadData + ADC);
        atomic_store(&(ThreadData + ADC)->Active, false);     // signal closed
    }
    FreeWBDynamicMemory();
    return NULL;
}
