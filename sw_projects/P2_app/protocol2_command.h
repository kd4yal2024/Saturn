#ifndef SATURN_PROTOCOL2_COMMAND_H
#define SATURN_PROTOCOL2_COMMAND_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define P2_GENERAL_PACKET_SIZE 60U
#define P2_PROTOCOL2_WIRE_DDC_COUNT 80U
#define P2_SATURN_HARDWARE_DDC_COUNT 10U
#define P2_SATURN_ADVERTISED_DDC_COUNT 10U
#define P2_GENERAL_WIDEBAND_COUNT 2U
#define P2_DDC_SPECIFIC_PACKET_SIZE 1444U
#define P2_PROTOCOL2_WIRE_ADC_COUNT 8U
#define P2_SATURN_ADC_COUNT 2U
#define P2_DUC_SPECIFIC_PACKET_SIZE 60U
#define P2_PROTOCOL2_WIRE_DAC_COUNT 4U
#define P2_SATURN_DAC_COUNT 1U
#define P2_PROTOCOL2_TX_ATTENUATOR_COUNT 3U
#define P2_HIGH_PRIORITY_PACKET_SIZE 1444U

typedef enum
{
    eP2GeneralPortDDCSpecific = 0,
    eP2GeneralPortDUCSpecific,
    eP2GeneralPortHighPriorityToSDR,
    eP2GeneralPortSpeakerAudio,
    eP2GeneralPortDUCIQ,
    eP2GeneralPortHighPriorityFromSDR,
    eP2GeneralPortMicAudio,
    eP2GeneralPortDDCIQ,
    eP2GeneralPortWideband,
    eP2GeneralPortCount
} EP2GeneralPort;

typedef struct
{
    uint32_t Sequence;
    uint16_t DDCSpecificPort;
    uint16_t DUCSpecificPort;
    uint16_t HighPriorityToSDRPort;
    uint16_t SpeakerAudioPort;
    uint16_t DUCIQPort;
    uint16_t HighPriorityFromSDRPort;
    uint16_t DDCIQBasePort;
    uint16_t MicAudioPort;
    uint16_t WidebandBasePort;
    uint8_t WidebandEnables;
    uint16_t WidebandSampleCount;
    uint8_t WidebandSampleSize;
    uint8_t WidebandUpdateRate;
    uint8_t WidebandPacketsPerFrame;
    uint16_t MinimumPWMWidth;
    uint16_t MaximumPWMWidth;
    bool TimestampEnabled;
    bool Vita49Enabled;
    bool FrequencyIsPhaseWord;
    bool WatchdogEnabled;
    bool PAEnabled;
    bool ApolloEnabled;
    uint8_t AlexEnableBits;
} TP2GeneralCommand;

// Domain-level action boundary between validated Protocol 2 commands and the
// concrete Saturn socket/register implementation. Every callback is required;
// apply validates the complete sink before invoking the first action.
typedef struct
{
    void (*SetPort)(void *Context, EP2GeneralPort Kind, uint8_t Index, uint16_t Port);
    void (*SetWideband)(void *Context, uint8_t Enables, uint16_t SampleCount,
                        uint8_t SampleSize, uint8_t UpdateRate, uint8_t PacketsPerFrame);
    void (*SetPWMWidths)(void *Context, uint16_t Minimum, uint16_t Maximum);
    void (*SetProtocolOptions)(void *Context, bool TimestampEnabled, bool Vita49Enabled,
                               bool FrequencyIsPhaseWord);
    void (*SetWatchdogEnabled)(void *Context, bool Enabled);
    void (*SetRadioOptions)(void *Context, bool PAEnabled, bool ApolloEnabled,
                            uint8_t AlexEnableBits);
} TP2GeneralActionSink;

typedef enum
{
    eP2DDCSourceADC1 = 0,
    eP2DDCSourceADC2,
    eP2DDCSourceTXSamples,
    eP2DDCSourceCount
} EP2DDCSource;

typedef struct
{
    bool Enabled;
    bool Interleaved;
    EP2DDCSource Source;
    uint16_t SampleRate;
    uint8_t SampleSize;
} TP2DDCConfig;

typedef struct
{
    uint32_t Sequence;
    uint8_t ADCCount;
    bool ADCDither[P2_SATURN_ADC_COUNT];
    bool ADCRandom[P2_SATURN_ADC_COUNT];
    uint16_t EnableMask;
    TP2DDCConfig DDC[P2_SATURN_HARDWARE_DDC_COUNT];
} TP2DDCSpecificCommand;

// Domain-level boundary for a validated receive/DDC-specific command. The
// concrete sink performs the register and stream-reconfiguration operations.
typedef struct
{
    void (*SetADCCount)(void *Context, uint8_t Count);
    void (*SetADCOptions)(void *Context, uint8_t ADCIndex, bool Dither, bool Random);
    void (*SetDDCConfig)(void *Context, uint8_t DDCIndex, const TP2DDCConfig *Config);
    void (*CommitDDCConfig)(void *Context);
} TP2DDCActionSink;

typedef struct
{
    bool EEREnabled;
    bool CWEnabled;
    bool ReverseKeys;
    bool IambicEnabled;
    bool SidetoneEnabled;
    bool ModeB;
    bool StrictSpacing;
    bool BreakIn;
    uint8_t SidetoneLevel;
    uint16_t SidetoneFrequencyHz;
    uint8_t KeyerSpeedWPM;
    uint8_t KeyerWeight;
    uint16_t HangDelayMs;
    uint8_t RFDelayMs;
    uint8_t RampPeriodMs;
} TP2DUCCWConfig;

typedef struct
{
    bool LineIn;
    bool MicBoost;
    bool MicPTTEnabled;
    bool MicPTTOnTip;
    bool MicBiasEnabled;
    bool BalancedMicInput;
    uint8_t LineInGain;
} TP2DUCMicConfig;

typedef struct
{
    uint32_t Sequence;
    uint8_t DACCount;
    uint16_t DUCSampleRate;
    uint8_t DUCSampleSize;
    uint16_t DUCPhaseShiftDegrees;
    TP2DUCCWConfig CW;
    TP2DUCMicConfig Mic;
    uint8_t TXAttenuation[P2_PROTOCOL2_TX_ATTENUATOR_COUNT];
} TP2DUCSpecificCommand;

// Domain-level boundary for a validated transmit/DUC-specific command. Only
// Saturn's implemented ADC attenuators are applied; all three Protocol 2 wire
// attenuation fields are nevertheless decoded and validated first.
typedef struct
{
    void (*SetCWConfig)(void *Context, const TP2DUCCWConfig *Config);
    void (*SetMicConfig)(void *Context, const TP2DUCMicConfig *Config);
    void (*SetTXAttenuation)(void *Context, uint8_t ADCIndex, uint8_t Attenuation);
} TP2DUCActionSink;

typedef struct
{
    bool Enabled;
    bool Dot;
    bool Dash;
} TP2HighPriorityCWXConfig;

typedef struct
{
    bool TransverterEnabled;
    bool SpeakerMuted;
    bool AutoTuneEnabled;
    uint8_t OpenCollectorBits;
    uint8_t UserOutputBits;
    uint8_t MercuryAttenuatorBits;
} TP2HighPriorityOutputConfig;

typedef struct
{
    uint16_t Alex1TXWord;
    uint16_t Alex1RXWord;
    uint16_t Alex0TXWord;
    uint16_t Alex0RXWord;
} TP2HighPriorityAlexConfig;

typedef struct
{
    uint32_t Sequence;
    bool Run;
    bool Transmit;
    bool PureSignal;
    TP2HighPriorityCWXConfig CWX;
    uint32_t DDCFrequency[P2_SATURN_HARDWARE_DDC_COUNT];
    uint32_t DUCFrequency;
    uint8_t DriveLevel;
    uint16_t ClientControlWord;
    uint16_t CATPort;
    TP2HighPriorityOutputConfig Outputs;
    TP2HighPriorityAlexConfig Alex;
    uint8_t RXAttenuation[P2_SATURN_ADC_COUNT];
} TP2HighPriorityCommand;

// Session policy is resolved only after controller and sequence checks. It
// keeps startup-handshake decisions separate from both wire decoding and the
// concrete Saturn register implementation.
typedef struct
{
    bool UpdateTXEnable;
    bool TXEnabled;
    bool TransmitActive;
    bool DisableCW;
    bool ApplyPayload;
} TP2HighPrioritySessionPolicy;

typedef struct
{
    void (*SetTXEnabled)(void *Context, bool Enabled);
    void (*SetMOX)(void *Context, bool Enabled);
    void (*DisableCW)(void *Context);
    void (*SetDDCFrequency)(void *Context, uint8_t DDCIndex, uint32_t Frequency);
    void (*SetDUCConfig)(void *Context, uint32_t Frequency, uint8_t DriveLevel);
    void (*SetClientControl)(void *Context, uint16_t ClientControlWord);
    void (*SetCATPort)(void *Context, uint16_t Port);
    void (*SetOutputs)(void *Context, const TP2HighPriorityOutputConfig *Config);
    void (*SetAlexConfig)(void *Context, const TP2HighPriorityAlexConfig *Config);
    void (*SetRXAttenuation)(void *Context, uint8_t ADCIndex, uint8_t Attenuation);
    void (*SetCWXConfig)(void *Context, const TP2HighPriorityCWXConfig *Config);
} TP2HighPriorityActionSink;

bool P2DecodeGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                            TP2GeneralCommand *Command);
bool P2ApplyGeneralCommand(const TP2GeneralCommand *Command,
                           const TP2GeneralActionSink *Sink, void *Context);
bool P2DecodeAndApplyGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                                    const TP2GeneralActionSink *Sink, void *Context);

bool P2DecodeDDCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                TP2DDCSpecificCommand *Command);
bool P2ApplyDDCSpecificCommand(const TP2DDCSpecificCommand *Command,
                               const TP2DDCActionSink *Sink, void *Context);
bool P2DecodeAndApplyDDCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                        const TP2DDCActionSink *Sink, void *Context);

bool P2DecodeDUCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                TP2DUCSpecificCommand *Command);
bool P2ApplyDUCSpecificCommand(const TP2DUCSpecificCommand *Command,
                               const TP2DUCActionSink *Sink, void *Context);
bool P2DecodeAndApplyDUCSpecificCommand(const uint8_t *Packet, size_t PacketLength,
                                        const TP2DUCActionSink *Sink, void *Context);

bool P2DecodeHighPriorityCommand(const uint8_t *Packet, size_t PacketLength,
                                 TP2HighPriorityCommand *Command);
bool P2ApplyHighPriorityCommand(const TP2HighPriorityCommand *Command,
                                const TP2HighPrioritySessionPolicy *Policy,
                                const TP2HighPriorityActionSink *Sink, void *Context);
bool P2DecodeAndApplyHighPriorityCommand(const uint8_t *Packet, size_t PacketLength,
                                         const TP2HighPrioritySessionPolicy *Policy,
                                         const TP2HighPriorityActionSink *Sink, void *Context);

#endif
