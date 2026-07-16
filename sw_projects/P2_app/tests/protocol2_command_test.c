#include "protocol2_command.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct
{
    unsigned int ActionCount;
    unsigned int PortActionCount;
    uint16_t Ports[eP2GeneralPortCount][P2_GENERAL_DDC_COUNT];
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
} TMockGeneralActions;

static void write_be_u16(uint8_t *Destination, uint16_t Value)
{
    Destination[0] = (uint8_t)(Value >> 8);
    Destination[1] = (uint8_t)Value;
}

static void write_be_u32(uint8_t *Destination, uint32_t Value)
{
    Destination[0] = (uint8_t)(Value >> 24);
    Destination[1] = (uint8_t)(Value >> 16);
    Destination[2] = (uint8_t)(Value >> 8);
    Destination[3] = (uint8_t)Value;
}

static void make_general_packet(uint8_t Packet[P2_GENERAL_PACKET_SIZE])
{
    memset(Packet, 0, P2_GENERAL_PACKET_SIZE);
    write_be_u32(Packet, 0x10203040U);
    Packet[4] = 0U;
    write_be_u16(Packet + 5, 12025U);
    write_be_u16(Packet + 7, 12026U);
    write_be_u16(Packet + 9, 12027U);
    write_be_u16(Packet + 11, 12028U);
    write_be_u16(Packet + 13, 12029U);
    write_be_u16(Packet + 15, 12030U);
    write_be_u16(Packet + 17, 13035U);
    write_be_u16(Packet + 19, 12031U);
    write_be_u16(Packet + 21, 14035U);
    Packet[23] = 0x03U;
    write_be_u16(Packet + 24, 512U);
    Packet[26] = 16U;
    Packet[27] = 25U;
    Packet[28] = 4U;
    write_be_u16(Packet + 33, 0x1234U);
    write_be_u16(Packet + 35, 0x5678U);
    Packet[37] = 0x0bU;
    Packet[38] = 0x01U;
    Packet[58] = 0x03U;
    Packet[59] = 0xa5U;
}

static void mock_set_port(void *Context, EP2GeneralPort Kind, uint8_t Index, uint16_t Port)
{
    TMockGeneralActions *Mock = Context;
    assert(Mock != NULL);
    assert(Kind < eP2GeneralPortCount);
    assert(Index < P2_GENERAL_DDC_COUNT);
    Mock->ActionCount++;
    Mock->PortActionCount++;
    Mock->Ports[Kind][Index] = Port;
}

static void mock_set_wideband(void *Context, uint8_t Enables, uint16_t SampleCount,
                              uint8_t SampleSize, uint8_t UpdateRate,
                              uint8_t PacketsPerFrame)
{
    TMockGeneralActions *Mock = Context;
    Mock->ActionCount++;
    Mock->WidebandEnables = Enables;
    Mock->WidebandSampleCount = SampleCount;
    Mock->WidebandSampleSize = SampleSize;
    Mock->WidebandUpdateRate = UpdateRate;
    Mock->WidebandPacketsPerFrame = PacketsPerFrame;
}

static void mock_set_pwm(void *Context, uint16_t Minimum, uint16_t Maximum)
{
    TMockGeneralActions *Mock = Context;
    Mock->ActionCount++;
    Mock->MinimumPWMWidth = Minimum;
    Mock->MaximumPWMWidth = Maximum;
}

static void mock_set_protocol_options(void *Context, bool TimestampEnabled,
                                      bool Vita49Enabled, bool FrequencyIsPhaseWord)
{
    TMockGeneralActions *Mock = Context;
    Mock->ActionCount++;
    Mock->TimestampEnabled = TimestampEnabled;
    Mock->Vita49Enabled = Vita49Enabled;
    Mock->FrequencyIsPhaseWord = FrequencyIsPhaseWord;
}

static void mock_set_watchdog(void *Context, bool Enabled)
{
    TMockGeneralActions *Mock = Context;
    Mock->ActionCount++;
    Mock->WatchdogEnabled = Enabled;
}

static void mock_set_radio_options(void *Context, bool PAEnabled, bool ApolloEnabled,
                                   uint8_t AlexEnableBits)
{
    TMockGeneralActions *Mock = Context;
    Mock->ActionCount++;
    Mock->PAEnabled = PAEnabled;
    Mock->ApolloEnabled = ApolloEnabled;
    Mock->AlexEnableBits = AlexEnableBits;
}

static const TP2GeneralActionSink MockSink = {
    .SetPort = mock_set_port,
    .SetWideband = mock_set_wideband,
    .SetPWMWidths = mock_set_pwm,
    .SetProtocolOptions = mock_set_protocol_options,
    .SetWatchdogEnabled = mock_set_watchdog,
    .SetRadioOptions = mock_set_radio_options,
};

static void test_general_packet_decodes_all_fields(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE];
    TP2GeneralCommand Command;

    make_general_packet(Packet);
    assert(P2DecodeGeneralCommand(Packet, sizeof(Packet), &Command));
    assert(Command.Sequence == 0x10203040U);
    assert(Command.DDCSpecificPort == 12025U);
    assert(Command.DUCSpecificPort == 12026U);
    assert(Command.HighPriorityToSDRPort == 12027U);
    assert(Command.HighPriorityFromSDRPort == 12028U);
    assert(Command.SpeakerAudioPort == 12029U);
    assert(Command.DUCIQPort == 12030U);
    assert(Command.DDCIQBasePort == 13035U);
    assert(Command.MicAudioPort == 12031U);
    assert(Command.WidebandBasePort == 14035U);
    assert(Command.WidebandEnables == 0x03U);
    assert(Command.WidebandSampleCount == 512U);
    assert(Command.WidebandSampleSize == 16U);
    assert(Command.WidebandUpdateRate == 25U);
    assert(Command.WidebandPacketsPerFrame == 4U);
    assert(Command.MinimumPWMWidth == 0x1234U);
    assert(Command.MaximumPWMWidth == 0x5678U);
    assert(Command.TimestampEnabled);
    assert(Command.Vita49Enabled);
    assert(Command.FrequencyIsPhaseWord);
    assert(Command.WatchdogEnabled);
    assert(Command.PAEnabled);
    assert(Command.ApolloEnabled);
    assert(Command.AlexEnableBits == 0xa5U);
}

static void test_general_command_applies_through_mock_boundary(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE];
    TMockGeneralActions Mock = {0};

    make_general_packet(Packet);
    assert(P2DecodeAndApplyGeneralCommand(Packet, sizeof(Packet), &MockSink, &Mock));
    assert(Mock.PortActionCount == 19U);
    assert(Mock.ActionCount == 24U);
    assert(Mock.Ports[eP2GeneralPortDDCSpecific][0] == 12025U);
    assert(Mock.Ports[eP2GeneralPortHighPriorityFromSDR][0] == 12028U);
    assert(Mock.Ports[eP2GeneralPortDDCIQ][0] == 13035U);
    assert(Mock.Ports[eP2GeneralPortDDCIQ][9] == 13044U);
    assert(Mock.Ports[eP2GeneralPortWideband][0] == 14035U);
    assert(Mock.Ports[eP2GeneralPortWideband][1] == 14036U);
    assert(Mock.WidebandSampleCount == 512U);
    assert(Mock.MinimumPWMWidth == 0x1234U);
    assert(Mock.MaximumPWMWidth == 0x5678U);
    assert(Mock.TimestampEnabled && Mock.Vita49Enabled && Mock.FrequencyIsPhaseWord);
    assert(Mock.WatchdogEnabled && Mock.PAEnabled && Mock.ApolloEnabled);
    assert(Mock.AlexEnableBits == 0xa5U);
}

static void test_zero_base_ports_preserve_default_port_semantics(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE];
    TMockGeneralActions Mock = {0};

    make_general_packet(Packet);
    write_be_u16(Packet + 17, 0U);
    write_be_u16(Packet + 21, 0U);
    assert(P2DecodeAndApplyGeneralCommand(Packet, sizeof(Packet), &MockSink, &Mock));
    assert(Mock.Ports[eP2GeneralPortDDCIQ][0] == 0U);
    assert(Mock.Ports[eP2GeneralPortDDCIQ][9] == 0U);
    assert(Mock.Ports[eP2GeneralPortWideband][0] == 0U);
    assert(Mock.Ports[eP2GeneralPortWideband][1] == 0U);
}

static void test_malformed_general_packets_never_reach_actions(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE + 1U];
    TMockGeneralActions Mock = {0};
    TP2GeneralCommand Command;
    size_t Index;

    make_general_packet(Packet);
    assert(!P2DecodeAndApplyGeneralCommand(NULL, P2_GENERAL_PACKET_SIZE, &MockSink, &Mock));
    assert(!P2DecodeAndApplyGeneralCommand(Packet, P2_GENERAL_PACKET_SIZE - 1U, &MockSink, &Mock));
    assert(!P2DecodeAndApplyGeneralCommand(Packet, P2_GENERAL_PACKET_SIZE + 1U, &MockSink, &Mock));
    Packet[4] = 2U;
    assert(!P2DecodeAndApplyGeneralCommand(Packet, P2_GENERAL_PACKET_SIZE, &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);

    memset(&Command, 0xa5, sizeof(Command));
    assert(!P2DecodeGeneralCommand(Packet, P2_GENERAL_PACKET_SIZE, &Command));
    for(Index = 0U; Index < sizeof(Command); Index++)
        assert(((const uint8_t *)&Command)[Index] == 0U);
}

static void test_incrementing_port_ranges_cannot_wrap(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE];
    TMockGeneralActions Mock = {0};
    TP2GeneralCommand Command;

    make_general_packet(Packet);
    write_be_u16(Packet + 17, UINT16_MAX - P2_GENERAL_DDC_COUNT + 2U);
    assert(!P2DecodeAndApplyGeneralCommand(Packet, sizeof(Packet), &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);

    make_general_packet(Packet);
    write_be_u16(Packet + 21, UINT16_MAX);
    assert(!P2DecodeAndApplyGeneralCommand(Packet, sizeof(Packet), &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);

    memset(&Command, 0, sizeof(Command));
    Command.DDCIQBasePort = UINT16_MAX;
    assert(!P2ApplyGeneralCommand(&Command, &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);
}

static void test_incomplete_action_sink_is_rejected_before_first_action(void)
{
    uint8_t Packet[P2_GENERAL_PACKET_SIZE];
    TP2GeneralCommand Command;
    TP2GeneralActionSink IncompleteSink = MockSink;
    TMockGeneralActions Mock = {0};

    make_general_packet(Packet);
    assert(P2DecodeGeneralCommand(Packet, sizeof(Packet), &Command));
    IncompleteSink.SetRadioOptions = NULL;
    assert(!P2ApplyGeneralCommand(&Command, &IncompleteSink, &Mock));
    assert(Mock.ActionCount == 0U);
    assert(!P2ApplyGeneralCommand(NULL, &MockSink, &Mock));
    assert(!P2ApplyGeneralCommand(&Command, NULL, &Mock));
}

int main(void)
{
    test_general_packet_decodes_all_fields();
    test_general_command_applies_through_mock_boundary();
    test_zero_base_ports_preserve_default_port_semantics();
    test_malformed_general_packets_never_reach_actions();
    test_incrementing_port_ranges_cannot_wrap();
    test_incomplete_action_sink_is_rejected_before_first_action();
    puts("protocol2 command boundary tests passed");
    return 0;
}
