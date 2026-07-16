#ifndef SATURN_PROTOCOL2_COMMAND_H
#define SATURN_PROTOCOL2_COMMAND_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define P2_GENERAL_PACKET_SIZE 60U
#define P2_GENERAL_DDC_COUNT 10U
#define P2_GENERAL_WIDEBAND_COUNT 2U

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

bool P2DecodeGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                            TP2GeneralCommand *Command);
bool P2ApplyGeneralCommand(const TP2GeneralCommand *Command,
                           const TP2GeneralActionSink *Sink, void *Context);
bool P2DecodeAndApplyGeneralCommand(const uint8_t *Packet, size_t PacketLength,
                                    const TP2GeneralActionSink *Sink, void *Context);

#endif
