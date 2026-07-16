/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// incomingDDCspecific.c:
//
// handle handle "DDC specific" message
//
//////////////////////////////////////////////////////////////


#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "IncomingDDCSpecific.h"
#include <errno.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include "../common/saturnregisters.h"
#include "../common/p23_perf_telemetry.h"
#include "OutDDCIQ.h"
#include "controller_lease.h"
#include <pthread.h>
#include <syscall.h>

_Static_assert(P2_SATURN_HARDWARE_DDC_COUNT == VNUMDDC,
               "Protocol 2 DDC command count must match Saturn hardware");

static void ApplyDDCCount(void *Context, uint8_t Count)
{
  (void)Context;
  SetADCCount(Count);
}

static void ApplyADCOptions(void *Context, uint8_t ADCIndex, bool Dither, bool Random)
{
  (void)Context;
  SetADCOptions((ADCIndex == 0U) ? eADC1 : eADC2, false, Dither, Random);
}

static void ApplyDDCConfig(void *Context, uint8_t DDCIndex, const TP2DDCConfig *Config)
{
  EADCSelect ADC;

  (void)Context;
  switch(Config->Source)
  {
    case eP2DDCSourceADC1: ADC = eADC1; break;
    case eP2DDCSourceADC2: ADC = eADC2; break;
    case eP2DDCSourceTXSamples: ADC = eTXSamples; break;
    case eP2DDCSourceCount:
    default: return;
  }

  SetDDCSampleSize(DDCIndex, Config->SampleSize);
  SetDDCADC(DDCIndex, ADC);
  SetP2SampleRate(DDCIndex, Config->Enabled, Config->SampleRate, Config->Interleaved);
  P23PerfTelemetrySetDDCConfig(DDCIndex, Config->Enabled,
                              Config->Interleaved, Config->SampleRate);
}

static void CommitDDCConfig(void *Context)
{
  (void)Context;
  if(WriteP2DDCRateRegister())
    HandlerCheckDDCSettings();
}

static const TP2DDCActionSink DDCActionSink = {
  .SetADCCount = ApplyDDCCount,
  .SetADCOptions = ApplyADCOptions,
  .SetDDCConfig = ApplyDDCConfig,
  .CommitDDCConfig = CommitDDCConfig,
};



//
// listener thread for incoming DDC specific packets
//
void *IncomingDDCSpecific(void *arg)                    // listener thread
{
  struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
  struct sockaddr_in addr_from;                         // holds MAC address of source of incoming messages
  uint8_t UDPInBuffer[VDDCSPECIFICSIZE];                // incoming buffer
  struct iovec iovecinst;                               // iovcnt buffer - 1 for each outgoing buffer
  struct msghdr datagram;                               // multiple incoming message header
  int size;                                             // UDP datagram length
  TP2DDCSpecificCommand Command;

  ThreadData = (struct ThreadSocketData *)arg;
  atomic_store(&ThreadData->Active, true);
  printf("spinning up DDC specific thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
  //
  // main processing loop
  //
  while(!atomic_load(&ExitRequested))
  {
    if(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT)
    {
      printf("DDC specific request change port\n");
      close(GetThreadSocketFD(ThreadData));
      if(MakeSocket(ThreadData, 0) != 0)
      {
        perror("MakeSocket, DDC specific");
        atomic_store(&ThreadError, true);
        break;
      }
      atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
    }

    memset(&iovecinst, 0, sizeof(struct iovec));
    memset(&datagram, 0, sizeof(datagram));
    iovecinst.iov_base = &UDPInBuffer;                  // set buffer for incoming message number i
    iovecinst.iov_len = VDDCSPECIFICSIZE;
    datagram.msg_iov = &iovecinst;
    datagram.msg_iovlen = 1;
    datagram.msg_name = &addr_from;
    datagram.msg_namelen = sizeof(addr_from);
    size = recvmsg(atomic_load(&ThreadData->Socketid), &datagram, 0);   // get one message. If it times out, ges size=-1
    if(size < 0 && errno != EAGAIN)
    {
      perror("recvfrom, DDC Specific");
      atomic_store(&ThreadError, true);
      break;
    }
    if((datagram.msg_flags & MSG_TRUNC) != 0)
      continue;
    if(size == VDDCSPECIFICSIZE)
    {
      if(!P2DecodeDDCSpecificCommand(UDPInBuffer, (size_t)size, &Command))
        continue;
      if(!ControllerLeaseMatches(&addr_from))
        continue;
      if(!P2ApplyDDCSpecificCommand(&Command, &DDCActionSink, NULL))
        continue;
      atomic_store(&NewMessageReceived, true);
      if(UseDebug)
        printf("DDC specific packet received\n");
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
