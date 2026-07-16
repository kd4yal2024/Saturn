/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// incomingDUCspecific.c:
//
// handle handle "DUC specific" message
// (also shown as "TX specific" in the protocol document)
//
//////////////////////////////////////////////////////////////


#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "IncomingDUCSpecific.h"
#include <errno.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include "../common/saturnregisters.h"
#include <pthread.h>
#include <syscall.h>
#include "controller_lease.h"
#include "protocol2_command.h"

_Static_assert(P2_DUC_SPECIFIC_PACKET_SIZE == VDUCSPECIFICSIZE,
               "Protocol 2 DUC packet size must match the listener buffer");
_Static_assert(P2_SATURN_ADC_COUNT == 2U,
               "DUC attenuation mapping expects Saturn's two ADCs");

static void ApplyCWConfig(void *Context, const TP2DUCCWConfig *Config)
{
    (void)Context;
    SetCWIambicKeyer(Config->KeyerSpeedWPM, Config->KeyerWeight,
                    Config->ReverseKeys, Config->ModeB, Config->StrictSpacing,
                    Config->IambicEnabled, Config->BreakIn);
    SetCWSidetoneEnabled(Config->SidetoneEnabled);
    EnableCW(Config->CWEnabled, Config->BreakIn);
    SetCWSidetoneVol(Config->SidetoneLevel);
    SetCWSidetoneFrequency(Config->SidetoneFrequencyHz);
    SetCWPTTDelay(Config->RFDelayMs);
    SetCWHangTime(Config->HangDelayMs);
    if(Config->RampPeriodMs != 0U)
        InitialiseCWKeyerRamp(true, (uint32_t)Config->RampPeriodMs * 1000U);
}

static void ApplyMicConfig(void *Context, const TP2DUCMicConfig *Config)
{
    (void)Context;
    // The codec requires source selection before boost and line-gain changes.
    SetMicLineInput(Config->LineIn);
    SetMicBoost(Config->MicBoost);
    SetOrionMicOptions(Config->MicPTTOnTip, Config->MicBiasEnabled,
                       Config->MicPTTEnabled);
    SetBalancedMicInput(Config->BalancedMicInput);
    SetCodecLineInGain(Config->LineInGain);
}

static void ApplyTXAttenuation(void *Context, uint8_t ADCIndex, uint8_t Attenuation)
{
    const EADCSelect ADC = (ADCIndex == 0U) ? eADC1 : eADC2;

    (void)Context;
    SetADCAttenuator(ADC, Attenuation, false, true);
}

static const TP2DUCActionSink DUCActionSink = {
    .SetCWConfig = ApplyCWConfig,
    .SetMicConfig = ApplyMicConfig,
    .SetTXAttenuation = ApplyTXAttenuation,
};

//
// listener thread for incoming DUC specific packets
//
void *IncomingDUCSpecific(void *arg)                    // listener thread
{ 
    struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
    struct sockaddr_in addr_from;                         // holds MAC address of source of incoming messages
    uint8_t UDPInBuffer[VDUCSPECIFICSIZE];                // incoming buffer
    struct iovec iovecinst;                               // iovcnt buffer - 1 for each outgoing buffer
    struct msghdr datagram;                               // multiple incoming message header
    int size;                                             // UDP datagram length
    TP2DUCSpecificCommand Command;

    ThreadData = (struct ThreadSocketData *)arg;
    atomic_store(&ThreadData->Active, true);
    printf("spinning up DUC specific thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
    //
    // main processing loop
    //
    while(!atomic_load(&ExitRequested))
    {
      if(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT)
      {
          printf("DUC specific request change port\n");
          close(GetThreadSocketFD(ThreadData));
          if(MakeSocket(ThreadData, 0) != 0)
          {
              perror("MakeSocket, DUC specific");
              atomic_store(&ThreadError, true);
              break;
          }
          atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
      }

      memset(&iovecinst, 0, sizeof(struct iovec));
      memset(&datagram, 0, sizeof(datagram));
      iovecinst.iov_base = &UDPInBuffer;                  // set buffer for incoming message number i
      iovecinst.iov_len = VDUCSPECIFICSIZE;
      datagram.msg_iov = &iovecinst;
      datagram.msg_iovlen = 1;
      datagram.msg_name = &addr_from;
      datagram.msg_namelen = sizeof(addr_from);
      size = recvmsg(atomic_load(&ThreadData->Socketid), &datagram, 0);   // get one message. If it times out, ges size=-1
      if(size < 0 && errno != EAGAIN)
      {
          perror("recvfrom, DUC specific");
          atomic_store(&ThreadError, true);
          break;
      }
      if((datagram.msg_flags & MSG_TRUNC) != 0)
          continue;
      if(size == VDUCSPECIFICSIZE)
      {
          if(!P2DecodeDUCSpecificCommand(UDPInBuffer, (size_t)size, &Command))
              continue;
          if(!ControllerLeaseMatches(&addr_from))
              continue;
          if(!P2ApplyDUCSpecificCommand(&Command, &DUCActionSink, NULL))
              continue;
          atomic_store(&NewMessageReceived, true);
          if(UseDebug)
              printf("DUC packet received\n");
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

