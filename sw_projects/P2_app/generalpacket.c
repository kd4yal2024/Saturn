/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// generalpacket.c:
//
// handle "general packet to SDR" message
//
//////////////////////////////////////////////////////////////


#include "threaddata.h"
#include <stddef.h>
#include <stdio.h>
#include "generalpacket.h"
#include "Outwideband.h"
#include "../common/saturnregisters.h"


atomic_bool HW_Timer_Enable = true;

_Static_assert(P2_SATURN_HARDWARE_DDC_COUNT == VNUMDDC,
               "Protocol 2 general command DDC count must match Saturn hardware");

static void ApplyGeneralPort(void *Context, EP2GeneralPort Kind, uint8_t Index, uint16_t Port)
{
  (void)Context;
  switch(Kind)
  {
    case eP2GeneralPortDDCSpecific: SetPort(VPORTDDCSPECIFIC, Port); break;
    case eP2GeneralPortDUCSpecific: SetPort(VPORTDUCSPECIFIC, Port); break;
    case eP2GeneralPortHighPriorityToSDR: SetPort(VPORTHIGHPRIORITYTOSDR, Port); break;
    case eP2GeneralPortSpeakerAudio: SetPort(VPORTSPKRAUDIO, Port); break;
    case eP2GeneralPortDUCIQ: SetPort(VPORTDUCIQ, Port); break;
    case eP2GeneralPortHighPriorityFromSDR: SetPort(VPORTHIGHPRIORITYFROMSDR, Port); break;
    case eP2GeneralPortMicAudio: SetPort(VPORTMICAUDIO, Port); break;
    case eP2GeneralPortDDCIQ: SetPort(VPORTDDCIQ0 + Index, Port); break;
    case eP2GeneralPortWideband: SetPort(VPORTWIDEBAND0 + Index, Port); break;
    case eP2GeneralPortCount: break;
  }
}

static void ApplyGeneralWideband(void *Context, uint8_t Enables, uint16_t SampleCount,
                                 uint8_t SampleSize, uint8_t UpdateRate,
                                 uint8_t PacketsPerFrame)
{
  (void)Context;
  SetWidebandParams(Enables, SampleCount, SampleSize, UpdateRate, PacketsPerFrame);
}

static void ApplyGeneralPWMWidths(void *Context, uint16_t Minimum, uint16_t Maximum)
{
  (void)Context;
  SetMinPWMWidth(Minimum);
  SetMaxPWMWidth(Maximum);
}

static void ApplyGeneralProtocolOptions(void *Context, bool TimestampEnabled,
                                        bool Vita49Enabled, bool FrequencyIsPhaseWord)
{
  (void)Context;
  EnableTimeStamp(TimestampEnabled);
  EnableVITA49(Vita49Enabled);
  SetFreqPhaseWord(FrequencyIsPhaseWord);
}

static void ApplyGeneralWatchdog(void *Context, bool Enabled)
{
  (void)Context;
  atomic_store(&HW_Timer_Enable, Enabled);
}

static void ApplyGeneralRadioOptions(void *Context, bool PAEnabled, bool ApolloEnabled,
                                     uint8_t AlexEnableBits)
{
  (void)Context;
  SetPAEnabled(PAEnabled);
  SetApolloEnabled(ApolloEnabled);
  SetAlexEnabled(AlexEnableBits);
}

static const TP2GeneralActionSink GeneralActionSink = {
  .SetPort = ApplyGeneralPort,
  .SetWideband = ApplyGeneralWideband,
  .SetPWMWidths = ApplyGeneralPWMWidths,
  .SetProtocolOptions = ApplyGeneralProtocolOptions,
  .SetWatchdogEnabled = ApplyGeneralWatchdog,
  .SetRadioOptions = ApplyGeneralRadioOptions,
};

int HandleGeneralPacket(const uint8_t *PacketBuffer, size_t PacketLength)
{
  return P2DecodeAndApplyGeneralCommand(PacketBuffer, PacketLength,
                                        &GeneralActionSink, NULL) ? 0 : -1;
}
