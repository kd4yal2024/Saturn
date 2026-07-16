#include "protocol2_command.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct
{
    unsigned int ActionCount;
    unsigned int PayloadActionCount;
    unsigned int TXEnableActionCount;
    unsigned int MOXActionCount;
    unsigned int DisableCWActionCount;
    unsigned int DDCActionCount;
    unsigned int RXAttenuationActionCount;
    bool TXEnabled;
    bool MOXEnabled;
    uint32_t DDCFrequency[P2_SATURN_HARDWARE_DDC_COUNT];
    uint32_t DUCFrequency;
    uint8_t DriveLevel;
    uint16_t ClientControlWord;
    uint16_t CATPort;
    TP2HighPriorityOutputConfig Outputs;
    TP2HighPriorityAlexConfig Alex;
    uint8_t RXAttenuation[P2_SATURN_ADC_COUNT];
    TP2HighPriorityCWXConfig CWX;
} TMockHighPriorityActions;

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

static void make_high_priority_packet(uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE])
{
    uint8_t Index;

    memset(Packet, 0, P2_HIGH_PRIORITY_PACKET_SIZE);
    write_be_u32(Packet, 0x10203040U);
    Packet[4] = 0x83U;
    Packet[5] = 0x07U;
    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
        write_be_u32(Packet + 9U + ((size_t)Index * 4U), 0x10000000U + Index);
    write_be_u32(Packet + 329, 0xcafebabeU);
    Packet[345] = 255U;
    write_be_u16(Packet + 1396, 0x1234U);
    write_be_u16(Packet + 1398, 5000U);
    Packet[1400] = 0x07U;
    Packet[1401] = 0xfeU;
    Packet[1402] = 0x0fU;
    Packet[1403] = 0x0fU;
    write_be_u16(Packet + 1428, 0x0701U);
    write_be_u16(Packet + 1430, 0x2345U);
    write_be_u16(Packet + 1432, 0x6789U);
    write_be_u16(Packet + 1434, 0xabcdU);
    Packet[1442] = 17U;
    Packet[1443] = 5U;
}

static void mock_set_tx_enabled(void *Context, bool Enabled)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->TXEnableActionCount++;
    Mock->TXEnabled = Enabled;
}

static void mock_set_mox(void *Context, bool Enabled)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->MOXActionCount++;
    Mock->MOXEnabled = Enabled;
}

static void mock_disable_cw(void *Context)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->DisableCWActionCount++;
}

static void mock_set_ddc_frequency(void *Context, uint8_t DDCIndex,
                                   uint32_t Frequency)
{
    TMockHighPriorityActions *Mock = Context;
    assert(DDCIndex < P2_SATURN_HARDWARE_DDC_COUNT);
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->DDCActionCount++;
    Mock->DDCFrequency[DDCIndex] = Frequency;
}

static void mock_set_duc_config(void *Context, uint32_t Frequency, uint8_t DriveLevel)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->DUCFrequency = Frequency;
    Mock->DriveLevel = DriveLevel;
}

static void mock_set_client_control(void *Context, uint16_t ClientControlWord)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->ClientControlWord = ClientControlWord;
}

static void mock_set_cat_port(void *Context, uint16_t Port)
{
    TMockHighPriorityActions *Mock = Context;
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->CATPort = Port;
}

static void mock_set_outputs(void *Context, const TP2HighPriorityOutputConfig *Config)
{
    TMockHighPriorityActions *Mock = Context;
    assert(Config != NULL);
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->Outputs = *Config;
}

static void mock_set_alex_config(void *Context, const TP2HighPriorityAlexConfig *Config)
{
    TMockHighPriorityActions *Mock = Context;
    assert(Config != NULL);
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->Alex = *Config;
}

static void mock_set_rx_attenuation(void *Context, uint8_t ADCIndex,
                                    uint8_t Attenuation)
{
    TMockHighPriorityActions *Mock = Context;
    assert(ADCIndex < P2_SATURN_ADC_COUNT);
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->RXAttenuationActionCount++;
    Mock->RXAttenuation[ADCIndex] = Attenuation;
}

static void mock_set_cwx_config(void *Context, const TP2HighPriorityCWXConfig *Config)
{
    TMockHighPriorityActions *Mock = Context;
    assert(Config != NULL);
    Mock->ActionCount++;
    Mock->PayloadActionCount++;
    Mock->CWX = *Config;
}

static const TP2HighPriorityActionSink MockSink = {
    .SetTXEnabled = mock_set_tx_enabled,
    .SetMOX = mock_set_mox,
    .DisableCW = mock_disable_cw,
    .SetDDCFrequency = mock_set_ddc_frequency,
    .SetDUCConfig = mock_set_duc_config,
    .SetClientControl = mock_set_client_control,
    .SetCATPort = mock_set_cat_port,
    .SetOutputs = mock_set_outputs,
    .SetAlexConfig = mock_set_alex_config,
    .SetRXAttenuation = mock_set_rx_attenuation,
    .SetCWXConfig = mock_set_cwx_config,
};

static TP2HighPrioritySessionPolicy active_tx_policy(void)
{
    TP2HighPrioritySessionPolicy Policy = {0};

    Policy.UpdateTXEnable = true;
    Policy.TXEnabled = true;
    Policy.TransmitActive = true;
    Policy.ApplyPayload = true;
    return Policy;
}

static void test_high_priority_packet_decodes_all_saturn_fields(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE];
    TP2HighPriorityCommand Command;
    uint8_t Index;

    make_high_priority_packet(Packet);
    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    assert(Command.Sequence == 0x10203040U);
    assert(Command.Run && Command.Transmit && Command.PureSignal);
    assert(Command.CWX.Enabled && Command.CWX.Dot && Command.CWX.Dash);
    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
        assert(Command.DDCFrequency[Index] == 0x10000000U + Index);
    assert(Command.DUCFrequency == 0xcafebabeU);
    assert(Command.DriveLevel == 255U);
    assert(Command.ClientControlWord == 0x1234U);
    assert(Command.CATPort == 5000U);
    assert(Command.Outputs.TransverterEnabled);
    assert(Command.Outputs.SpeakerMuted);
    assert(Command.Outputs.AutoTuneEnabled);
    assert(Command.Outputs.OpenCollectorBits == 0x7fU);
    assert(Command.Outputs.UserOutputBits == 0x0fU);
    assert(Command.Outputs.MercuryAttenuatorBits == 0x0fU);
    assert(Command.Alex.Alex1TXWord == 0x0701U);
    assert(Command.Alex.Alex1RXWord == 0x2345U);
    assert(Command.Alex.Alex0TXWord == 0x6789U);
    assert(Command.Alex.Alex0RXWord == 0xabcdU);
    assert(Command.RXAttenuation[0] == 5U);
    assert(Command.RXAttenuation[1] == 17U);
}

static void test_active_tx_applies_complete_payload(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE];
    TP2HighPriorityCommand Command;
    TP2HighPrioritySessionPolicy Policy = active_tx_policy();
    TMockHighPriorityActions Mock = {0};

    make_high_priority_packet(Packet);
    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    assert(P2ApplyHighPriorityCommand(&Command, &Policy, &MockSink, &Mock));
    assert(Mock.ActionCount == 20U);
    assert(Mock.PayloadActionCount == 18U);
    assert(Mock.TXEnableActionCount == 1U && Mock.TXEnabled);
    assert(Mock.MOXActionCount == 1U && Mock.MOXEnabled);
    assert(Mock.DisableCWActionCount == 0U);
    assert(Mock.DDCActionCount == P2_SATURN_HARDWARE_DDC_COUNT);
    assert(Mock.DDCFrequency[9] == 0x10000009U);
    assert(Mock.DUCFrequency == 0xcafebabeU && Mock.DriveLevel == 255U);
    assert(Mock.ClientControlWord == 0x1234U && Mock.CATPort == 5000U);
    assert(Mock.Outputs.AutoTuneEnabled && Mock.Outputs.OpenCollectorBits == 0x7fU);
    assert(Mock.Alex.Alex0RXWord == 0xabcdU);
    assert(Mock.RXAttenuationActionCount == P2_SATURN_ADC_COUNT);
    assert(Mock.RXAttenuation[0] == 5U && Mock.RXAttenuation[1] == 17U);
    assert(Mock.CWX.Enabled && Mock.CWX.Dot && Mock.CWX.Dash);
}

static void test_run_zero_with_transmit_bit_only_performs_safe_stop(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE];
    TP2HighPriorityCommand Command;
    TP2HighPrioritySessionPolicy Policy = {0};
    TMockHighPriorityActions Mock = {0};

    make_high_priority_packet(Packet);
    Packet[4] = 0x82U;
    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    assert(!Command.Run && !Command.Transmit && Command.PureSignal);

    Policy.UpdateTXEnable = true;
    Policy.DisableCW = true;
    assert(P2ApplyHighPriorityCommand(&Command, &Policy, &MockSink, &Mock));
    assert(Mock.ActionCount == 3U);
    assert(Mock.PayloadActionCount == 0U);
    assert(Mock.TXEnableActionCount == 1U && !Mock.TXEnabled);
    assert(Mock.MOXActionCount == 1U && !Mock.MOXEnabled);
    assert(Mock.DisableCWActionCount == 1U);
}

static void test_incomplete_handshake_cannot_key_mox(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE];
    TP2HighPriorityCommand Command;
    TP2HighPrioritySessionPolicy Policy = {0};
    TMockHighPriorityActions Mock = {0};

    make_high_priority_packet(Packet);
    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    Policy.ApplyPayload = true;
    assert(P2ApplyHighPriorityCommand(&Command, &Policy, &MockSink, &Mock));
    assert(Mock.TXEnableActionCount == 0U);
    assert(Mock.MOXActionCount == 1U && !Mock.MOXEnabled);
    assert(Mock.PayloadActionCount == 18U);
}

static void test_malformed_packets_never_reach_actions(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE + 1U];
    TP2HighPriorityCommand Command;
    TP2HighPrioritySessionPolicy Policy = active_tx_policy();
    TMockHighPriorityActions Mock = {0};
    size_t Index;

    make_high_priority_packet(Packet);
    assert(!P2DecodeAndApplyHighPriorityCommand(NULL, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE - 1U,
                                                 &Policy, &MockSink, &Mock));
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE + 1U,
                                                 &Policy, &MockSink, &Mock));

    Packet[4] |= 0x04U;
    memset(&Command, 0xa5, sizeof(Command));
    assert(!P2DecodeHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE, &Command));
    for(Index = 0U; Index < sizeof(Command); Index++)
        assert(((const uint8_t *)&Command)[Index] == 0U);

    make_high_priority_packet(Packet);
    Packet[5] |= 0x08U;
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));
    for(Index = 6U; Index <= 8U; Index++)
    {
        make_high_priority_packet(Packet);
        Packet[Index] = 1U;
        assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                     &Policy, &MockSink, &Mock));
    }

    make_high_priority_packet(Packet);
    Packet[1400] |= 0x08U;
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));
    make_high_priority_packet(Packet);
    Packet[1401] |= 0x01U;
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));
    make_high_priority_packet(Packet);
    Packet[1402] |= 0x10U;
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));
    make_high_priority_packet(Packet);
    Packet[1403] |= 0x10U;
    assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                 &Policy, &MockSink, &Mock));

    for(Index = 1442U; Index <= 1443U; Index++)
    {
        make_high_priority_packet(Packet);
        Packet[Index] = 32U;
        assert(!P2DecodeAndApplyHighPriorityCommand(Packet, P2_HIGH_PRIORITY_PACKET_SIZE,
                                                     &Policy, &MockSink, &Mock));
    }
    assert(Mock.ActionCount == 0U);
}

static void test_invalid_policy_sink_and_forged_command_are_rejected(void)
{
    uint8_t Packet[P2_HIGH_PRIORITY_PACKET_SIZE];
    TP2HighPriorityCommand Command;
    TP2HighPrioritySessionPolicy Policy = active_tx_policy();
    TP2HighPriorityActionSink IncompleteSink = MockSink;
    TMockHighPriorityActions Mock = {0};

    make_high_priority_packet(Packet);
    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    IncompleteSink.SetAlexConfig = NULL;
    assert(!P2ApplyHighPriorityCommand(&Command, &Policy, &IncompleteSink, &Mock));
    assert(Mock.ActionCount == 0U);

    Policy.TransmitActive = true;
    Command.Transmit = false;
    assert(!P2ApplyHighPriorityCommand(&Command, &Policy, &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);

    assert(P2DecodeHighPriorityCommand(Packet, sizeof(Packet), &Command));
    Policy = active_tx_policy();
    Command.RXAttenuation[0] = 32U;
    assert(!P2ApplyHighPriorityCommand(&Command, &Policy, &MockSink, &Mock));
    assert(Mock.ActionCount == 0U);
    assert(!P2ApplyHighPriorityCommand(NULL, &Policy, &MockSink, &Mock));
    assert(!P2ApplyHighPriorityCommand(&Command, NULL, &MockSink, &Mock));
    assert(!P2ApplyHighPriorityCommand(&Command, &Policy, NULL, &Mock));
}

int main(void)
{
    test_high_priority_packet_decodes_all_saturn_fields();
    test_active_tx_applies_complete_payload();
    test_run_zero_with_transmit_bit_only_performs_safe_stop();
    test_incomplete_handshake_cannot_key_mox();
    test_malformed_packets_never_reach_actions();
    test_invalid_policy_sink_and_forged_command_are_rejected();
    puts("protocol2 high-priority boundary tests passed");
    return 0;
}
