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
#define P2_DUC_MODE_OFFSET 5U
#define P2_DUC_MIC_OPTIONS_OFFSET 50U
#define P2_DUC_TX_ATTENUATION_BASE_OFFSET 57U
#define P2_DUC_SIDETONE_LEVEL_MAX 127U
#define P2_DUC_KEYER_SPEED_MAX 60U
#define P2_DUC_KEYER_WEIGHT_MIN 33U
#define P2_DUC_KEYER_WEIGHT_MAX 66U
#define P2_DUC_RAMP_PERIOD_MIN_MS 5U
#define P2_DUC_RAMP_PERIOD_MAX_MS 10U
#define P2_DUC_HANG_DELAY_MAX_MS 1023U
#define P2_DUC_LINE_IN_GAIN_MAX 31U
#define P2_DUC_TX_ATTENUATION_MAX 31U
#define P2_DUC_MIC_OPTIONS_MASK 0x3fU
#define P2_DUC_PHASE_SHIFT_MAX_DEGREES 359U

_Static_assert((P2_PROTOCOL2_WIRE_DDC_COUNT % 8U) == 0U,
               "Protocol 2 DDC enable bitmap must contain whole bytes");
_Static_assert(P2_SATURN_HARDWARE_DDC_COUNT <= 16U,
               "Saturn DDC command model uses a 16-bit hardware enable mask");
_Static_assert(P2_SATURN_ADVERTISED_DDC_COUNT == P2_SATURN_HARDWARE_DDC_COUNT,
               "Saturn discovery must advertise the tested hardware DDC count");
_Static_assert(P2_SATURN_ADC_COUNT <= P2_PROTOCOL2_WIRE_ADC_COUNT,
               "Saturn ADC count cannot exceed the Protocol 2 wire maximum");
_Static_assert(P2_SATURN_DAC_COUNT <= P2_PROTOCOL2_WIRE_DAC_COUNT,
               "Saturn DAC count cannot exceed the Protocol 2 wire maximum");
_Static_assert(P2_SATURN_ADC_COUNT <= P2_PROTOCOL2_TX_ATTENUATOR_COUNT,
               "Saturn ADC count cannot exceed DUC attenuation fields");

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

static bool Protocol2SampleRateIsSupported(uint16_t SampleRate)
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
    return Protocol2SampleRateIsSupported(Config->SampleRate) &&
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

static bool DUCCWConfigIsValid(const TP2DUCCWConfig *Config)
{
    if((Config == NULL) || (Config->SidetoneLevel > P2_DUC_SIDETONE_LEVEL_MAX) ||
       (Config->KeyerSpeedWPM > P2_DUC_KEYER_SPEED_MAX) ||
       (Config->HangDelayMs > P2_DUC_HANG_DELAY_MAX_MS))
        return false;
    if(Config->CWEnabled && (Config->KeyerWeight == 0U))
        return false;
    if((Config->KeyerWeight != 0U) &&
       ((Config->KeyerWeight < P2_DUC_KEYER_WEIGHT_MIN) ||
        (Config->KeyerWeight > P2_DUC_KEYER_WEIGHT_MAX)))
        return false;
    return (Config->RampPeriodMs == 0U) ||
           ((Config->RampPeriodMs >= P2_DUC_RAMP_PERIOD_MIN_MS) &&
            (Config->RampPeriodMs <= P2_DUC_RAMP_PERIOD_MAX_MS));
}

static bool DUCMicConfigIsValid(const TP2DUCMicConfig *Config)
{
    return (Config != NULL) && (Config->LineInGain <= P2_DUC_LINE_IN_GAIN_MAX);
}

static bool DUCSpecificCommandIsValid(const TP2DUCSpecificCommand *Command)
{
    uint8_t Index;

    if((Command == NULL) || (Command->DACCount > P2_SATURN_DAC_COUNT) ||
       ((Command->DUCSampleRate != 0U) &&
        !Protocol2SampleRateIsSupported(Command->DUCSampleRate)) ||
       ((Command->DUCSampleSize != 0U) && (Command->DUCSampleSize != 24U)) ||
       (Command->DUCPhaseShiftDegrees > P2_DUC_PHASE_SHIFT_MAX_DEGREES) ||
       !DUCCWConfigIsValid(&Command->CW) || !DUCMicConfigIsValid(&Command->Mic))
        return false;
    for(Index = 0U; Index < P2_PROTOCOL2_TX_ATTENUATOR_COUNT; Index++)
    {
        if(Command->TXAttenuation[Index] > P2_DUC_TX_ATTENUATION_MAX)
            return false;
    }
    return true;
}

bool P2DecodeDUCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                TP2DUCSpecificCommand *Command)
{
    uint8_t Index;
    uint8_t MicOptions;
    uint8_t Mode;

    if(Command == NULL)
        return false;
    memset(Command, 0, sizeof(*Command));
    if((Packet == NULL) || (PacketLength != P2_DUC_SPECIFIC_PACKET_SIZE))
        return false;

    MicOptions = Packet[P2_DUC_MIC_OPTIONS_OFFSET];
    if((MicOptions & (uint8_t)~P2_DUC_MIC_OPTIONS_MASK) != 0U)
        return false;

    Command->Sequence = rd_be_u32(Packet);
    Command->DACCount = Packet[4];
    Mode = Packet[P2_DUC_MODE_OFFSET];
    Command->CW.EEREnabled = (Mode & 0x01U) != 0U;
    Command->CW.CWEnabled = (Mode & 0x02U) != 0U;
    Command->CW.ReverseKeys = (Mode & 0x04U) != 0U;
    Command->CW.IambicEnabled = (Mode & 0x08U) != 0U;
    Command->CW.SidetoneEnabled = (Mode & 0x10U) != 0U;
    Command->CW.ModeB = (Mode & 0x20U) != 0U;
    Command->CW.StrictSpacing = (Mode & 0x40U) != 0U;
    Command->CW.BreakIn = (Mode & 0x80U) != 0U;
    Command->CW.SidetoneLevel = Packet[6];
    Command->CW.SidetoneFrequencyHz = rd_be_u16(Packet + 7);
    Command->CW.KeyerSpeedWPM = Packet[9];
    Command->CW.KeyerWeight = Packet[10];
    Command->CW.HangDelayMs = rd_be_u16(Packet + 11);
    Command->CW.RFDelayMs = Packet[13];
    Command->DUCSampleRate = rd_be_u16(Packet + 14);
    Command->DUCSampleSize = Packet[16];
    Command->CW.RampPeriodMs = Packet[17];
    Command->DUCPhaseShiftDegrees = rd_be_u16(Packet + 26);

    Command->Mic.LineIn = (MicOptions & 0x01U) != 0U;
    Command->Mic.MicBoost = (MicOptions & 0x02U) != 0U;
    Command->Mic.MicPTTEnabled = (MicOptions & 0x04U) == 0U;
    Command->Mic.MicPTTOnTip = (MicOptions & 0x08U) != 0U;
    Command->Mic.MicBiasEnabled = (MicOptions & 0x10U) != 0U;
    Command->Mic.BalancedMicInput = (MicOptions & 0x20U) != 0U;
    Command->Mic.LineInGain = Packet[51];

    // Store attenuation by Protocol 2 ADC index: byte 59 is ADC0, byte 58 is
    // ADC1, and byte 57 is ADC2. Saturn applies ADC0 and ADC1 only.
    for(Index = 0U; Index < P2_PROTOCOL2_TX_ATTENUATOR_COUNT; Index++)
    {
        Command->TXAttenuation[Index] =
            Packet[P2_DUC_TX_ATTENUATION_BASE_OFFSET +
                   (P2_PROTOCOL2_TX_ATTENUATOR_COUNT - 1U - Index)];
    }

    if(!DUCSpecificCommandIsValid(Command))
    {
        memset(Command, 0, sizeof(*Command));
        return false;
    }
    return true;
}

static bool DUCActionSinkIsComplete(const TP2DUCActionSink *Sink)
{
    return (Sink != NULL) && (Sink->SetCWConfig != NULL) &&
           (Sink->SetMicConfig != NULL) && (Sink->SetTXAttenuation != NULL);
}

bool P2ApplyDUCSpecificCommand(const TP2DUCSpecificCommand *Command,
                               const TP2DUCActionSink *Sink, void *Context)
{
    uint8_t Index;

    if(!DUCSpecificCommandIsValid(Command) || !DUCActionSinkIsComplete(Sink))
        return false;

    Sink->SetCWConfig(Context, &Command->CW);
    Sink->SetMicConfig(Context, &Command->Mic);
    // Preserve the listener's established wire order: Protocol ADC1 (Saturn
    // eADC2/reference) is updated before Protocol ADC0 (Saturn eADC1/feedback).
    for(Index = P2_SATURN_ADC_COUNT; Index > 0U; Index--)
    {
        const uint8_t ADCIndex = (uint8_t)(Index - 1U);
        Sink->SetTXAttenuation(Context, ADCIndex, Command->TXAttenuation[ADCIndex]);
    }
    return true;
}

bool P2DecodeAndApplyDUCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                        const TP2DUCActionSink *Sink, void *Context)
{
    TP2DUCSpecificCommand Command;

    if(!P2DecodeDUCSpecificCommand(Packet, PacketLength, &Command))
        return false;
    return P2ApplyDUCSpecificCommand(&Command, Sink, Context);
}
