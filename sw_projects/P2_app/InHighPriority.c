/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// InHighPriority.c:
//
// handle "incoming high priority" message
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include "InHighPriority.h"
#include "controller_lease.h"
#include "protocol2_command.h"
#include "protocol2_control.h"
#include <errno.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include "../common/saturnregisters.h"
#include "../common/hwaccess.h"                   // low level access
#include "../common/version.h"
#include "cathandler.h"
#include "AriesATU.h"
#include <pthread.h>
#include <syscall.h>


extern uint32_t LODebugDDC1Frequency;                   // -x debug mode: LO frequency for DDC1
extern bool InterleavedDDCDebugMode;                    // true if interleaved DDC for debug are allowed

_Static_assert(P2_HIGH_PRIORITY_PACKET_SIZE == VHIGHPRIOTIYTOSDRSIZE,
               "Protocol 2 high-priority size must match the listener buffer");
_Static_assert(P2_SATURN_ADC_COUNT == 2U,
               "High-priority attenuation mapping expects Saturn's two ADCs");

typedef struct
{
  unsigned int FPGAVersion;
} TP2HighPriorityActionContext;

static void ApplyTXEnabled(void *Context, bool Enabled)
{
  (void)Context;
  SetTXEnable(Enabled);
}

static void ApplyMOX(void *Context, bool Enabled)
{
  (void)Context;
  SetMOX(Enabled);
}

static void ApplyDisableCW(void *Context)
{
  (void)Context;
  EnableCW(false, false);
}

static void ApplyDDCFrequency(void *Context, uint8_t DDCIndex, uint32_t Frequency)
{
  (void)Context;
  if(InterleavedDDCDebugMode && (DDCIndex == 1U))
    SetDDCFrequency(1, LODebugDDC1Frequency, false);
  else
    SetDDCFrequency(DDCIndex, Frequency, true);
}

static void ApplyDUCConfig(void *Context, uint32_t Frequency, uint8_t DriveLevel)
{
  (void)Context;
  SetDUCFrequency(Frequency, true);
  SetAriesTXFrequency(Frequency);
  SetTXDriveLevel(DriveLevel);
}

static void ApplyClientControl(void *Context, uint16_t ClientControlWord)
{
  (void)Context;
  SetClientControlWord(ClientControlWord);
}

static void ApplyCATPort(void *Context, uint16_t Port)
{
  (void)Context;
  if(Port != 0U)
    SetupCATPort(Port);
  else if(CATHandlerActive())
    ShutdownCATHandler();
}

static void ApplyOutputs(void *Context, const TP2HighPriorityOutputConfig *Config)
{
  (void)Context;
  SetXvtrEnable(Config->TransverterEnabled);
  SetSpkrMute(Config->SpeakerMuted);
  SetOpenCollectorOutputs(Config->OpenCollectorBits);
  SetUserOutputBits(Config->UserOutputBits);
}

static void ApplyAlexConfig(void *Context, const TP2HighPriorityAlexConfig *Config)
{
  TP2HighPriorityActionContext *ActionContext = Context;
  uint16_t TXAntennaBits;
  uint16_t Word;

  TXAntennaBits = (Config->Alex1TXWord >> 8) & 0x0007U;
  if((ActionContext->FPGAVersion >= 12U) && (TXAntennaBits != 0U))
  {
    Word = Config->Alex1TXWord;
    SetAriesAlexTXWord(Word);
    if(atomic_load(&AriesATUActive))
      Word = (Word & 0xF8FFU) | 0x0100U;
    AlexManualTXFilters(Word, true);

    Word = Config->Alex0TXWord;
    SetAriesAlexRXWord(Word);
    if(atomic_load(&AriesATUActive))
      Word = (Word & 0xF8FFU) | 0x0100U;
    AlexManualTXFilters(Word, false);
  }
  else if(ActionContext->FPGAVersion >= 12U)
  {
    AlexManualTXFilters(Config->Alex0TXWord, true);
    AlexManualTXFilters(Config->Alex0TXWord, false);
  }
  else
  {
    AlexManualTXFilters(Config->Alex0TXWord, false);
  }

  AlexManualRXFilters(Config->Alex1RXWord, 2);
  AlexManualRXFilters(Config->Alex0RXWord, 0);
}

static void ApplyRXAttenuation(void *Context, uint8_t ADCIndex, uint8_t Attenuation)
{
  const EADCSelect ADC = (ADCIndex == 0U) ? eADC1 : eADC2;

  (void)Context;
  SetADCAttenuator(ADC, Attenuation, true, false);
}

static void ApplyCWXConfig(void *Context, const TP2HighPriorityCWXConfig *Config)
{
  (void)Context;
  SetCWXBits(Config->Enabled, Config->Dash, Config->Dot);
}

static const TP2HighPriorityActionSink HighPriorityActionSink = {
  .SetTXEnabled = ApplyTXEnabled,
  .SetMOX = ApplyMOX,
  .DisableCW = ApplyDisableCW,
  .SetDDCFrequency = ApplyDDCFrequency,
  .SetDUCConfig = ApplyDUCConfig,
  .SetClientControl = ApplyClientControl,
  .SetCATPort = ApplyCATPort,
  .SetOutputs = ApplyOutputs,
  .SetAlexConfig = ApplyAlexConfig,
  .SetRXAttenuation = ApplyRXAttenuation,
  .SetCWXConfig = ApplyCWXConfig,
};


//
// listener thread for incoming high priority packets
//
void *IncomingHighPriority(void *arg)                   // listener thread
{
  struct ThreadSocketData *ThreadData;                  // socket etc data for this thread
  struct sockaddr_in addr_from;                         // holds MAC address of source of incoming messages
  uint8_t UDPInBuffer[VHIGHPRIOTIYTOSDRSIZE];           // incoming buffer
  struct iovec iovecinst;                               // iovcnt buffer - 1 for each outgoing buffer
  struct msghdr datagram;                               // multiple incoming message header
  int size;                                             // UDP datagram length
  uint32_t MissingPackets;
  bool HighPriorityStreamLogged = false;
  TP2SequenceTracker SequenceTracker = {0};
  TP2HighPriorityCommand Command;
  TP2HighPrioritySessionPolicy Policy;
  TP2HighPriorityActionContext ActionContext;
  ESoftwareID FPGASWID;                                 // preprod/release etc


  ThreadData = (struct ThreadSocketData *)arg;
  atomic_store(&ThreadData->Active, true);
  printf("spinning up high priority incoming thread with port %u, pid=%ld\n", (unsigned int)atomic_load(&ThreadData->Portid), syscall(SYS_gettid));
  ActionContext.FPGAVersion = GetFirmwareVersion(&FPGASWID);

  //
  // main processing loop
  //
  while(!atomic_load(&ExitRequested))
  {
    if(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT)
    {
      printf("High priority request change port\n");
      if(ThreadSocketIsSharedAlias(ThreadData))
      {
        // Shared aliases must not close/rebind the owner socket.
        struct ThreadSocketBindingSnapshot Binding;
        if(GetThreadSocketBinding(ThreadData, &Binding) &&
           (Binding.ThreadSocketfd > 0) &&
           (Binding.ThreadSocketfd != Binding.OwnerSocketfd))
        {
          close(Binding.ThreadSocketfd);
          atomic_store(&ThreadData->Socketid, 0);
        }
      }
      else
      {
        int Socketfd = atomic_load(&ThreadData->Socketid);
        if(Socketfd > 0)
          close(Socketfd);
        if(MakeSocket(ThreadData, 0) != 0)
        {
          perror("MakeSocket, high priority");
          atomic_store(&ThreadError, true);
          break;
        }
      }
      atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
    }

    memset(&iovecinst, 0, sizeof(struct iovec));
    memset(&datagram, 0, sizeof(datagram));
    iovecinst.iov_base = &UDPInBuffer;                  // set buffer for incoming message number i
    iovecinst.iov_len = VHIGHPRIOTIYTOSDRSIZE;
    datagram.msg_iov = &iovecinst;
    datagram.msg_iovlen = 1;
    datagram.msg_name = &addr_from;
    datagram.msg_namelen = sizeof(addr_from);
    {
      int Socketfd = GetThreadSocketFD(ThreadData);
      if(Socketfd <= 0)
      {
        usleep(1000);
        continue;
      }
      size = recvmsg(Socketfd, &datagram, 0);   // get one message. If it times out, ges size=-1
    }
    if(size < 0 && errno != EAGAIN)
    {
      perror("recvfrom, high priority");
      printf("error number = %d\n", errno);
      atomic_store(&ThreadError, true);
      break;
    }

    if((datagram.msg_flags & MSG_TRUNC) != 0)
      continue;

    //
    // if correct packet, process it
    //
    if(size == VHIGHPRIOTIYTOSDRSIZE)
    {
      bool HandshakeReady;
      bool WasActive;

      if(!P2DecodeHighPriorityCommand(UDPInBuffer, (size_t)size, &Command))
        continue;
      if(!ControllerLeaseMatches(&addr_from))
        continue;

      // StartBitReceived is cleared on run=0 and by the inactivity watchdog.
      // The next accepted controller session therefore gets a fresh sequence
      // epoch even if the new client starts again at sequence zero.
      if(!atomic_load(&StartBitReceived))
        P2SequenceReset(&SequenceTracker);
      // Control-packet rule, not the data-stream rule: Thetis sends every
      // high-priority control packet with sequence zero, so repeated sequence
      // numbers carry fresh state (frequency, drive, run) and must be applied.
      if(!P2ControlSequenceAccept(&SequenceTracker, Command.Sequence, &MissingPackets))
        continue;

      memset(&Policy, 0, sizeof(Policy));
      WasActive = atomic_load(&SDRActive);
      HandshakeReady = atomic_load(&ReplyAddressSet);
      if(Command.Run)
      {
        Policy.UpdateTXEnable = HandshakeReady;
        Policy.TXEnabled = HandshakeReady;
        Policy.TransmitActive = Command.Transmit && (WasActive || HandshakeReady);
        Policy.ApplyPayload = true;
      }
      else
      {
        Policy.UpdateTXEnable = true;
        Policy.DisableCW = true;
      }

      if(!P2ApplyHighPriorityCommand(&Command, &Policy, &HighPriorityActionSink,
                                     &ActionContext))
        continue;

      atomic_store(&NewMessageReceived, true);
      if((MissingPackets != 0U) && UseDebug)
        printf("High priority sequence gap: missing %u packet(s)\n", MissingPackets);
      if(!HighPriorityStreamLogged)
      {
        printf("STARTUP: High priority packet stream detected\n");
        HighPriorityStreamLogged = true;
      }

      if(Command.Run)
      {
        atomic_store(&StartBitReceived, true);
        MarkStartupRunBitSeen();
        if(HandshakeReady)
        {
          atomic_store(&SDRActive, true);
          MarkStartupHandshakeComplete();
        }
        atomic_store(&IsTXMode, Policy.TransmitActive);
      }
      else
      {
        atomic_store(&SDRActive, false);
        atomic_store(&IsTXMode, false);
        if(WasActive)
        {
          printf("set to inactive by client app\n");
          ResetStartupTraceFlags();
        }
        atomic_store(&StartBitReceived, false);
        ControllerLeaseRelease(&addr_from);
        P2SequenceReset(&SequenceTracker);
      }
    }
  }
//
// close down thread
//
  CloseThreadSocketIfOwned(ThreadData);    // close incoming data socket
  atomic_store(&ThreadData->Socketid, 0);
  atomic_store(&ThreadData->Active, false);     // indicate it is closed
  return NULL;
}
