/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// OutHighPriority.c:
//
// handle "outgoing high priority data" message
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
#include <sys/stat.h>
#include <time.h>
#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/byteio.h"
#include "../common/auxadc.h"
#include "../common/p23_perf_telemetry.h"
#include "LDGATU.h"

#define ADC_PEAK_TELEMETRY_ENABLE_FILE "/dev/shm/saturn_p23_adc_peak_telemetry.enabled"
#define ADC_PEAK_TELEMETRY_JSON_FILE "/dev/shm/saturn_p23_adc_peak_telemetry.json"

static void MaybeWriteADCPeakTelemetry(const char *AppName, uint16_t ADC1Peak, uint16_t ADC2Peak, uint8_t ADCOverflows)
{
  static bool TelemetryEnabled = false;
  static time_t LastFlagCheck = 0;
  static time_t LastWrite = 0;
  time_t Now;
  char TempPath[160];
  FILE *File;

  Now = time(NULL);
  if (Now == (time_t)-1)
    return;

  if ((LastFlagCheck == 0) || (Now != LastFlagCheck))
  {
    TelemetryEnabled = (access(ADC_PEAK_TELEMETRY_ENABLE_FILE, F_OK) == 0);
    LastFlagCheck = Now;
  }

  if (!TelemetryEnabled)
    return;

  if ((LastWrite != 0) && (Now == LastWrite))
    return;

  snprintf(TempPath, sizeof(TempPath), "%s.%ld.tmp", ADC_PEAK_TELEMETRY_JSON_FILE, (long)getpid());
  File = fopen(TempPath, "w");
  if (File == NULL)
    return;

  fprintf(
    File,
    "{\n"
    "  \"app\": \"%s\",\n"
    "  \"pid\": %ld,\n"
    "  \"timestamp_epoch\": %ld,\n"
    "  \"adc_overflows\": %u,\n"
    "  \"adc1_peak\": %u,\n"
    "  \"adc2_peak\": %u\n"
    "}\n",
    AppName,
    (long)getpid(),
    (long)Now,
    (unsigned int)ADCOverflows,
    (unsigned int)ADC1Peak,
    (unsigned int)ADC2Peak
  );

  if (fclose(File) != 0)
  {
    remove(TempPath);
    return;
  }

  if (rename(TempPath, ADC_PEAK_TELEMETRY_JSON_FILE) != 0)
  {
    remove(TempPath);
    return;
  }
  LastWrite = Now;
}


uint8_t GlobalFIFOOverflows = 0;             // FIFO overflow words
pthread_mutex_t g_fifo_overflow_mutex = PTHREAD_MUTEX_INITIALIZER;  // protect GlobalFIFOOverflows from race conditions



// this runs as its own thread to send outgoing data
// thread initiated after a "Start" command
// will be instructed to stop & exit by main loop setting enable_thread to 0
// this code signals thread terminated by setting active_thread = 0
//
void *OutgoingHighPriority(void *arg)
{
//
// variables for outgoing UDP frame
//
  struct iovec iovecinst;                                 // instance of iovec
  struct msghdr datagram;
  uint8_t UDPBuffer[VHIGHPRIOTIYFROMSDRSIZE];             // DDC frame buffer
  uint32_t SequenceCounter = 0;                           // UDP sequence count

  struct ThreadSocketData *ThreadData;            // socket etc data for this thread
  struct sockaddr_in DestAddr;                    // destination address for outgoing data
  bool InitError = false;
  int Error;
  int Socketfd;
  uint8_t Byte;                                   // data being encoded
  uint16_t Word;                                  // data being encoded
  unsigned int FIFOCount;
  uint32_t DDCFIFOSamples;
  uint32_t MicFIFOSamples;
  uint32_t DUCFIFOSamples;
  uint32_t SpeakerFIFOSamples;
  bool ATUTuneRequest = false;
  bool FIFOOverflow, FIFOUnderflow, FIFOOverThreshold;      // FIFO flags
  uint8_t FIFOOverflows;
  uint8_t ADCOverflows = 0;                       // set non zero if ADC overflows detected
  uint16_t ADC1MaxAmpl = 0;                       // latest ADC1 peak amplitude sample
  uint16_t ADC2MaxAmpl = 0;                       // latest ADC2 peak amplitude sample
  uint16_t PeakADC1MaxAmpl = 0;                   // peak hold for current message period
  uint16_t PeakADC2MaxAmpl = 0;                   // peak hold for current message period

//
// initialise. Create memory buffers and open DMA file devices
//
  ThreadData = (struct ThreadSocketData *)arg;
  atomic_store(&ThreadData->Active, true);
  printf("spinning up outgoing high priority with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));

//
// OK, now the main work
// thread commanded to transfer / stop transferring data by global bool SDRActive
// threat may also be commanded to close down and re-open its socket by command byte 
// VBITCHANGEPORT bit being set (shold only happen when not running)
//
  while (!InitError && !atomic_load(&ExitRequested))
  {
    while(!atomic_load(&SDRActive) && !atomic_load(&ExitRequested))
    {
      P23PerfTelemetrySetRuntimeFlags(
        atomic_load(&SDRActive),
        atomic_load(&IsTXMode),
        atomic_load(&ReplyAddressSet),
        atomic_load(&StartBitReceived),
        atomic_load(&ThreadError),
        atomic_load(&ExitRequested)
      );
      P23PerfTelemetrySetPureSignalEnabled(GetPureSignalEnabled());
      P23PerfTelemetrySetDieTempC(GetDieTempC());
      P23PerfTelemetryMaybeWrite();
      // Port rebinding is handled centrally by the p2app control plane.
      usleep(100);
    }
    //
    // if we get here, run has been initiated
    // initialise outgoing data packet
    //
    SequenceCounter = 0;
    printf("starting outgoing high priority data\n");
    pthread_mutex_lock(&g_reply_addr_mutex);
    memcpy(&DestAddr, &reply_addr, sizeof(struct sockaddr_in));           // local copy of PC destination address
    pthread_mutex_unlock(&g_reply_addr_mutex);
    memset(&iovecinst, 0, sizeof(struct iovec));
    memset(&datagram, 0, sizeof(datagram));
    memset(UDPBuffer, 0,sizeof(UDPBuffer));                      // clear the whole packet
    PeakADC1MaxAmpl = 0;
    PeakADC2MaxAmpl = 0;
    iovecinst.iov_base = UDPBuffer;
    iovecinst.iov_len = VHIGHPRIOTIYFROMSDRSIZE;
    datagram.msg_iov = &iovecinst;
    datagram.msg_iovlen = 1;
    datagram.msg_name = &DestAddr;                   // MAC addr & port to send to
    datagram.msg_namelen = sizeof(DestAddr);

    //
    // this is the main loop. SDR is running. transfer data;
    // also check for changes to DDC enabled, and DDC interleaved
    //
    // potential race conditions: thread execution order is underfined. 
    // when a DDC becomes enabled, its paired DDC may not know yet and may still be set to interleaved.
    // when a DDC is set to interleaved, the paired DDC may not have been disabled yet.
    //
    while(atomic_load(&SDRActive) && !InitError && !atomic_load(&ExitRequested))                // main loop
    {
      uint16_t SleepCount;                                      // counter for sending next message
      uint8_t PTTBits;                                          // PTT bits - and change means a new message needed
      // create the packet
      *(uint32_t *)UDPBuffer = htonl(SequenceCounter++);        // add sequence count
      ReadStatusRegister();
      PTTBits = (uint8_t)GetP2PTTKeyInputs();
      *(uint8_t *)(UDPBuffer+4) = PTTBits;
      ADCOverflows |= (uint8_t)GetADCOverflow(&ADC1MaxAmpl, &ADC2MaxAmpl);  // add in any new overflows
      PeakADC1MaxAmpl = (ADC1MaxAmpl > PeakADC1MaxAmpl) ? ADC1MaxAmpl : PeakADC1MaxAmpl;
      PeakADC2MaxAmpl = (ADC2MaxAmpl > PeakADC2MaxAmpl) ? ADC2MaxAmpl : PeakADC2MaxAmpl;
      *(uint8_t *)(UDPBuffer+5) = ADCOverflows;
      ADCOverflows = 0;                                         // and clear ready for next test
      wr_be_u16(UDPBuffer+39, PeakADC1MaxAmpl);                 // ADC1 peak hold
      wr_be_u16(UDPBuffer+41, PeakADC2MaxAmpl);                 // ADC2 peak hold
      MaybeWriteADCPeakTelemetry("p2", PeakADC1MaxAmpl, PeakADC2MaxAmpl, *(uint8_t *)(UDPBuffer+5));
      Word = (uint16_t)GetAnalogueIn(4);
      wr_be_u16(UDPBuffer+6, Word);                     // exciter power
      Word = (uint16_t)GetAnalogueIn(0);
      wr_be_u16(UDPBuffer+14, Word);                    // forward power
      Word = (uint16_t)GetAnalogueIn(1);
      wr_be_u16(UDPBuffer+22, Word);                    // reverse power
      Word = (uint16_t)GetAnalogueIn(5);
      wr_be_u16(UDPBuffer+49, Word);                    // supply voltage

      Word = (uint16_t)GetAnalogueIn(2);
      wr_be_u16(UDPBuffer+57, Word);                    // AIN3 user_analog1
      Word = (uint16_t)GetAnalogueIn(3);
      wr_be_u16(UDPBuffer+55, Word);                    // AIN4 user_analog2

      Byte = (uint8_t)GetUserIOBits();                  // user I/O bits
      *(uint8_t *)(UDPBuffer+59) = Byte;

//
// protocol V4.3: send FIFO depths and error states
// we can read a snapshot now, but under or overflows could have happened at other times too
// and they are cleared by the data transfer reads of the monitor channel
//
      FIFOOverflows = 0;
      ReadFIFOMonitorChannel(eRXDDCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &FIFOCount);				// read the DDC FIFO Depth register
      DDCFIFOSamples = FIFOCount;
      wr_be_u16(UDPBuffer+31, FIFOCount);                       // DDC samples
      if(FIFOOverThreshold)
        FIFOOverflows |= 0b00000001;

      ReadFIFOMonitorChannel(eMicCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &FIFOCount);				// read the mic FIFO Depth register

      Word = Word*4;                                            // 4 samples per FIFO location
      MicFIFOSamples = FIFOCount;
      wr_be_u16(UDPBuffer+33, FIFOCount);                       // mic samples
      if(FIFOOverThreshold)
        FIFOOverflows |= 0b00000010;

      ReadFIFOMonitorChannel(eTXDUCDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &FIFOCount);				// read the DUC FIFO Depth register
      Word = (Word*4)/3;                                        // 4/3 samples per FIFO location
      DUCFIFOSamples = FIFOCount;
      wr_be_u16(UDPBuffer+35, FIFOCount);                       // DUC samples
      if(FIFOUnderflow)
        FIFOOverflows |= 0b00000100;

      ReadFIFOMonitorChannel(eSpkCodecDMA, &FIFOOverflow, &FIFOOverThreshold, &FIFOUnderflow, &FIFOCount);				// read the speaker FIFO Depth register
      Word = Word*2;                                            // 2 samples per FIFO location
      SpeakerFIFOSamples = FIFOCount;
      wr_be_u16(UDPBuffer+37, FIFOCount);                       // speaker samples
      if(FIFOUnderflow)
        FIFOOverflows |= 0b00001000;

      pthread_mutex_lock(&g_fifo_overflow_mutex);
      FIFOOverflows |= GlobalFIFOOverflows;                   // copy in any bits set during normal data transfer
      GlobalFIFOOverflows = 0;                                // clear any overflows
      pthread_mutex_unlock(&g_fifo_overflow_mutex);
      *(uint8_t *)(UDPBuffer+30) = FIFOOverflows;
      P23PerfTelemetrySetRuntimeFlags(
        atomic_load(&SDRActive),
        atomic_load(&IsTXMode),
        atomic_load(&ReplyAddressSet),
        atomic_load(&StartBitReceived),
        atomic_load(&ThreadError),
        atomic_load(&ExitRequested)
      );
      P23PerfTelemetrySetPureSignalEnabled(GetPureSignalEnabled());
      P23PerfTelemetrySetFIFOSnapshot(DDCFIFOSamples, MicFIFOSamples, DUCFIFOSamples, SpeakerFIFOSamples, FIFOOverflows);
      P23PerfTelemetrySetADCSnapshot(PeakADC1MaxAmpl, PeakADC2MaxAmpl, *(uint8_t *)(UDPBuffer+5));
      if(*(uint8_t *)(UDPBuffer+5) != 0)
        P23PerfTelemetryCounterAdd(eP23PerfCounterADCOverflowEvents, 1U);
      PeakADC1MaxAmpl = 0;
      PeakADC2MaxAmpl = 0;
      FIFOOverflows = 0;
      Socketfd = GetThreadSocketFD(ThreadData);
      Error = sendmsg(Socketfd, &datagram, 0);


      //
      // get ATU bit and offer to LDG ATU handler
      // power requested if bit 2 is zero
      Byte = ((Byte >> 2) & 1) ^1;
      ATUTuneRequest = (bool)Byte;
      RequestATUTune(ATUTuneRequest);

      if(Error == -1)
      {
        printf("High Priority Send Error, errno=%d\n", errno);
        printf("socket id = %d\n", Socketfd);
        P23PerfTelemetryCounterAdd(eP23PerfCounterHighPrioritySendErrors, 1U);
        InitError=true;
      }
      else
      {
        P23PerfTelemetryCounterAdd(eP23PerfCounterHighPriorityPackets, 1U);
        P23PerfTelemetryCounterAdd(eP23PerfCounterHighPriorityBytes, (uint64_t)Error);
      }
      P23PerfTelemetrySetDieTempC(GetDieTempC());
      P23PerfTelemetryMaybeWrite();
      //
      // now we need to sleep for 1ms (in TX) or 200ms (not in TX)
      // BUT if any of the PTT or key inputs change, or ADC overflow detected, send a message immediately
      // so break up the 200ms period with smaller sleeps
      // thank you to Rick N1GP for recommending this approach
      //
      SleepCount = (MOXAsserted) ? 2 : 400;
      while ((SleepCount-- > 0) && !atomic_load(&ExitRequested))
      {
        ReadStatusRegister();
        if ((uint8_t)GetP2PTTKeyInputs() != PTTBits)
          break;
        ADCOverflows |= (uint8_t)GetADCOverflow(&ADC1MaxAmpl, &ADC2MaxAmpl);
        PeakADC1MaxAmpl = (ADC1MaxAmpl > PeakADC1MaxAmpl) ? ADC1MaxAmpl : PeakADC1MaxAmpl;
        PeakADC2MaxAmpl = (ADC2MaxAmpl > PeakADC2MaxAmpl) ? ADC2MaxAmpl : PeakADC2MaxAmpl;
        if(ADCOverflows != 0)
          break;
        usleep(500);
      }
    }
  }
//
// tidy shutdown of the thread
//
  if(InitError)                                           // if error, flag it to main program
    atomic_store(&ThreadError, true);
  printf("shutting down outgoing high priority thread\n");
  CloseThreadSocketIfOwned(ThreadData);
  atomic_store(&ThreadData->Active, false);     // signal closed
  return NULL;
}
