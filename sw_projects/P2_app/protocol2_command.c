#include "protocol2_command.h"

#include <string.h>

#include "../common/byteio.h"

#define P2_GENERAL_COMMAND_CODE 0U

static bool IncrementingPortRangeFits(uint16_t BasePort, uint8_t PortCount)
{
    return (BasePort == 0U) ||
           (((uint32_t)BasePort + (uint32_t)PortCount - 1U) <= UINT16_MAX);
}

static bool GeneralCommandIsValid(const TP2GeneralCommand *Command)
{
    return (Command != NULL) &&
           IncrementingPortRangeFits(Command->DDCIQBasePort, P2_GENERAL_DDC_COUNT) &&
           IncrementingPortRangeFits(Command->WidebandBasePort,
                                     P2_GENERAL_WIDEBAND_COUNT);
}

bool P2DecodeGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                            TP2GeneralCommand *Command)
{
    uint8_t Flags;
    uint8_t RadioOptions;

    if(Command == NULL)
        return false;
    memset(Command, 0, sizeof(*Command));
    if((Packet == NULL) || (PacketLength != P2_GENERAL_PACKET_SIZE) ||
       (Packet[4] != P2_GENERAL_COMMAND_CODE))
        return false;

    Command->Sequence = rd_be_u32(Packet);
    Command->DDCSpecificPort = rd_be_u16(Packet + 5);
    Command->DUCSpecificPort = rd_be_u16(Packet + 7);
    Command->HighPriorityToSDRPort = rd_be_u16(Packet + 9);
    Command->HighPriorityFromSDRPort = rd_be_u16(Packet + 11);
    Command->SpeakerAudioPort = rd_be_u16(Packet + 13);
    Command->DUCIQPort = rd_be_u16(Packet + 15);
    Command->DDCIQBasePort = rd_be_u16(Packet + 17);
    Command->MicAudioPort = rd_be_u16(Packet + 19);
    Command->WidebandBasePort = rd_be_u16(Packet + 21);
    Command->WidebandEnables = Packet[23];
    Command->WidebandSampleCount = rd_be_u16(Packet + 24);
    Command->WidebandSampleSize = Packet[26];
    Command->WidebandUpdateRate = Packet[27];
    Command->WidebandPacketsPerFrame = Packet[28];
    Command->MinimumPWMWidth = rd_be_u16(Packet + 33);
    Command->MaximumPWMWidth = rd_be_u16(Packet + 35);

    Flags = Packet[37];
    Command->TimestampEnabled = (Flags & 0x01U) != 0U;
    Command->Vita49Enabled = (Flags & 0x02U) != 0U;
    Command->FrequencyIsPhaseWord = (Flags & 0x08U) != 0U;
    Command->WatchdogEnabled = (Packet[38] & 0x01U) != 0U;

    RadioOptions = Packet[58];
    Command->PAEnabled = (RadioOptions & 0x01U) != 0U;
    Command->ApolloEnabled = (RadioOptions & 0x02U) != 0U;
    Command->AlexEnableBits = Packet[59];
    if(!GeneralCommandIsValid(Command))
    {
        memset(Command, 0, sizeof(*Command));
        return false;
    }
    return true;
}

static bool GeneralActionSinkIsComplete(const TP2GeneralActionSink *Sink)
{
    return (Sink != NULL) && (Sink->SetPort != NULL) && (Sink->SetWideband != NULL) &&
           (Sink->SetPWMWidths != NULL) && (Sink->SetProtocolOptions != NULL) &&
           (Sink->SetWatchdogEnabled != NULL) && (Sink->SetRadioOptions != NULL);
}

static uint16_t IncrementingPort(uint16_t BasePort, uint8_t Index)
{
    return (BasePort == 0U) ? 0U : (uint16_t)(BasePort + Index);
}

bool P2ApplyGeneralCommand(const TP2GeneralCommand *Command,
                           const TP2GeneralActionSink *Sink, void *Context)
{
    uint8_t Index;

    if(!GeneralCommandIsValid(Command) || !GeneralActionSinkIsComplete(Sink))
        return false;

    Sink->SetPort(Context, eP2GeneralPortDDCSpecific, 0U, Command->DDCSpecificPort);
    Sink->SetPort(Context, eP2GeneralPortDUCSpecific, 0U, Command->DUCSpecificPort);
    Sink->SetPort(Context, eP2GeneralPortHighPriorityToSDR, 0U,
                  Command->HighPriorityToSDRPort);
    Sink->SetPort(Context, eP2GeneralPortSpeakerAudio, 0U, Command->SpeakerAudioPort);
    Sink->SetPort(Context, eP2GeneralPortDUCIQ, 0U, Command->DUCIQPort);
    Sink->SetPort(Context, eP2GeneralPortHighPriorityFromSDR, 0U,
                  Command->HighPriorityFromSDRPort);
    Sink->SetPort(Context, eP2GeneralPortMicAudio, 0U, Command->MicAudioPort);

    for(Index = 0U; Index < P2_GENERAL_DDC_COUNT; Index++)
        Sink->SetPort(Context, eP2GeneralPortDDCIQ, Index,
                      IncrementingPort(Command->DDCIQBasePort, Index));
    for(Index = 0U; Index < P2_GENERAL_WIDEBAND_COUNT; Index++)
        Sink->SetPort(Context, eP2GeneralPortWideband, Index,
                      IncrementingPort(Command->WidebandBasePort, Index));

    Sink->SetWideband(Context, Command->WidebandEnables, Command->WidebandSampleCount,
                      Command->WidebandSampleSize, Command->WidebandUpdateRate,
                      Command->WidebandPacketsPerFrame);
    Sink->SetPWMWidths(Context, Command->MinimumPWMWidth, Command->MaximumPWMWidth);
    Sink->SetProtocolOptions(Context, Command->TimestampEnabled, Command->Vita49Enabled,
                             Command->FrequencyIsPhaseWord);
    Sink->SetWatchdogEnabled(Context, Command->WatchdogEnabled);
    Sink->SetRadioOptions(Context, Command->PAEnabled, Command->ApolloEnabled,
                          Command->AlexEnableBits);
    return true;
}

bool P2DecodeAndApplyGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                                    const TP2GeneralActionSink *Sink, void *Context)
{
    TP2GeneralCommand Command;

    if(!P2DecodeGeneralCommand(Packet, PacketLength, &Command))
        return false;
    return P2ApplyGeneralCommand(&Command, Sink, Context);
}
