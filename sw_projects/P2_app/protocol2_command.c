#include "protocol2_command.h"

#include <string.h>

#include "../common/byteio.h"

#define P2_GENERAL_COMMAND_CODE 0U
#define P2_DDC_CONFIG_BASE_OFFSET 17U
#define P2_DDC_CONFIG_STRIDE 6U
#define P2_DDC_ENABLE_BASE_OFFSET 7U
#define P2_DDC_SYNC_BASE_OFFSET 1363U
#define P2_DDC_SYNC_STRIDE 2U
#define P2_DDC_INTERLEAVE_PAIR_COUNT 4U

_Static_assert((P2_PROTOCOL2_WIRE_DDC_COUNT % 8U) == 0U,
               "Protocol 2 DDC enable bitmap must contain whole bytes");
_Static_assert(P2_SATURN_HARDWARE_DDC_COUNT <= 16U,
               "Saturn DDC command model uses a 16-bit hardware enable mask");
_Static_assert(P2_SATURN_ADVERTISED_DDC_COUNT == P2_SATURN_HARDWARE_DDC_COUNT,
               "Saturn discovery must advertise the tested hardware DDC count");
_Static_assert(P2_SATURN_ADC_COUNT <= P2_PROTOCOL2_WIRE_ADC_COUNT,
               "Saturn ADC count cannot exceed the Protocol 2 wire maximum");

static bool IncrementingPortRangeFits(uint16_t BasePort, uint8_t PortCount)
{
    return (BasePort == 0U) ||
           (((uint32_t)BasePort + (uint32_t)PortCount - 1U) <= UINT16_MAX);
}

static bool GeneralCommandIsValid(const TP2GeneralCommand *Command)
{
    return (Command != NULL) &&
           IncrementingPortRangeFits(Command->DDCIQBasePort,
                                     P2_SATURN_HARDWARE_DDC_COUNT) &&
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

    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
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

static bool DDCEnabledSampleRateIsSupported(uint16_t SampleRate)
{
    return (SampleRate == 48U) || (SampleRate == 96U) || (SampleRate == 192U) ||
           (SampleRate == 384U) || (SampleRate == 768U) || (SampleRate == 1536U);
}

static bool DDCConfigIsValid(const TP2DDCConfig *Config, uint8_t DDCIndex)
{
    if((Config == NULL) || (Config->Source < eP2DDCSourceADC1) ||
       (Config->Source >= eP2DDCSourceCount))
        return false;
    if(Config->Interleaved && (((DDCIndex & 1U) != 0U) ||
                               (DDCIndex >= (P2_DDC_INTERLEAVE_PAIR_COUNT * 2U))))
        return false;
    if(!Config->Enabled)
        return !Config->Interleaved;
    return DDCEnabledSampleRateIsSupported(Config->SampleRate) &&
           (Config->SampleSize == 24U);
}

static bool DDCSpecificCommandIsValid(const TP2DDCSpecificCommand *Command)
{
    uint8_t Index;
    const uint16_t SupportedEnableMask =
        (uint16_t)((1U << P2_SATURN_HARDWARE_DDC_COUNT) - 1U);

    if((Command == NULL) || (Command->ADCCount == 0U) ||
       (Command->ADCCount > P2_SATURN_ADC_COUNT) ||
       ((Command->EnableMask & (uint16_t)~SupportedEnableMask) != 0U))
        return false;

    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
    {
        const bool EnabledByMask =
            (Command->EnableMask & (uint16_t)(1U << Index)) != 0U;
        const bool EnabledByInterleave = ((Index & 1U) != 0U) &&
                                          Command->DDC[Index - 1U].Interleaved;

        if(!DDCConfigIsValid(&Command->DDC[Index], Index))
            return false;
        if(Command->DDC[Index].Enabled != (EnabledByMask || EnabledByInterleave))
            return false;
        if((Command->DDC[Index].Source == eP2DDCSourceADC2) &&
           (Command->ADCCount < P2_SATURN_ADC_COUNT))
            return false;
        if(Command->DDC[Index].Interleaved)
        {
            if(((uint8_t)(Index + 1U) >= P2_SATURN_HARDWARE_DDC_COUNT) ||
               !Command->DDC[Index + 1U].Enabled)
                return false;
        }
    }
    return true;
}

bool P2DecodeDDCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                TP2DDCSpecificCommand *Command)
{
    uint8_t Index;
    uint8_t Pair;

    if(Command == NULL)
        return false;
    memset(Command, 0, sizeof(*Command));
    if((Packet == NULL) || (PacketLength != P2_DDC_SPECIFIC_PACKET_SIZE))
        return false;

    // Bytes 7..16 are the Protocol 2 enable bitmap for DDC0..79. Saturn
    // implements DDC0..9; reject requests for the remaining wire-level DDCs.
    for(Index = 2U; Index < (P2_PROTOCOL2_WIRE_DDC_COUNT / 8U); Index++)
    {
        if(Packet[P2_DDC_ENABLE_BASE_OFFSET + Index] != 0U)
            return false;
    }

    Command->Sequence = rd_be_u32(Packet);
    Command->ADCCount = Packet[4];
    for(Index = 0U; Index < P2_SATURN_ADC_COUNT; Index++)
    {
        Command->ADCDither[Index] = (Packet[5] & (uint8_t)(1U << Index)) != 0U;
        Command->ADCRandom[Index] = (Packet[6] & (uint8_t)(1U << Index)) != 0U;
    }
    Command->EnableMask = rd_le_u16(Packet + P2_DDC_ENABLE_BASE_OFFSET);

    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
    {
        const size_t Offset = P2_DDC_CONFIG_BASE_OFFSET +
                              ((size_t)Index * P2_DDC_CONFIG_STRIDE);
        Command->DDC[Index].Enabled =
            (Command->EnableMask & (uint16_t)(1U << Index)) != 0U;
        Command->DDC[Index].Source = (EP2DDCSource)Packet[Offset];
        Command->DDC[Index].SampleRate = rd_be_u16(Packet + Offset + 1U);
        Command->DDC[Index].SampleSize = Packet[Offset + 5U];
    }

    // Saturn implements the four legacy even/odd interleave pairs. A matching
    // sync mask marks the even DDC interleaved and makes its odd partner active.
    for(Pair = 0U; Pair < P2_DDC_INTERLEAVE_PAIR_COUNT; Pair++)
    {
        const uint8_t EvenDDC = (uint8_t)(Pair * 2U);
        const uint8_t OddDDC = (uint8_t)(EvenDDC + 1U);
        const uint8_t ExpectedMask = (uint8_t)(1U << OddDDC);
        const size_t SyncOffset = P2_DDC_SYNC_BASE_OFFSET +
                                  ((size_t)Pair * P2_DDC_SYNC_STRIDE);

        if(Packet[SyncOffset] == ExpectedMask)
        {
            Command->DDC[EvenDDC].Interleaved = true;
            Command->DDC[OddDDC].Enabled = true;
        }
    }

    if(!DDCSpecificCommandIsValid(Command))
    {
        memset(Command, 0, sizeof(*Command));
        return false;
    }
    return true;
}

static bool DDCActionSinkIsComplete(const TP2DDCActionSink *Sink)
{
    return (Sink != NULL) && (Sink->SetADCCount != NULL) &&
           (Sink->SetADCOptions != NULL) && (Sink->SetDDCConfig != NULL) &&
           (Sink->CommitDDCConfig != NULL);
}

bool P2ApplyDDCSpecificCommand(const TP2DDCSpecificCommand *Command,
                               const TP2DDCActionSink *Sink, void *Context)
{
    uint8_t Index;

    if(!DDCSpecificCommandIsValid(Command) || !DDCActionSinkIsComplete(Sink))
        return false;

    Sink->SetADCCount(Context, Command->ADCCount);
    for(Index = 0U; Index < P2_SATURN_ADC_COUNT; Index++)
        Sink->SetADCOptions(Context, Index, Command->ADCDither[Index],
                            Command->ADCRandom[Index]);
    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
        Sink->SetDDCConfig(Context, Index, &Command->DDC[Index]);
    Sink->CommitDDCConfig(Context);
    return true;
}

bool P2DecodeAndApplyDDCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                        const TP2DDCActionSink *Sink, void *Context)
{
    TP2DDCSpecificCommand Command;

    if(!P2DecodeDDCSpecificCommand(Packet, PacketLength, &Command))
        return false;
    return P2ApplyDDCSpecificCommand(&Command, Sink, Context);
}
