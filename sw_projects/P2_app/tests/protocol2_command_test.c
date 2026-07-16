#include "protocol2_command.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct
{
    unsigned int ActionCount;
    unsigned int PortActionCount;
    uint16_t Ports[eP2GeneralPortCount][P2_SATURN_HARDWARE_DDC_COUNT];
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

typedef struct
{
    unsigned int ActionCount;
    unsigned int ADCOptionActionCount;
    unsigned int DDCActionCount;
    unsigned int CommitActionCount;
    uint8_t ADCCount;
    bool ADCDither[P2_SATURN_ADC_COUNT];
    bool ADCRandom[P2_SATURN_ADC_COUNT];
    TP2DDCConfig DDC[P2_SATURN_HARDWARE_DDC_COUNT];
} TMockDDCActions;

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

static void write_le_u16(uint8_t *Destination, uint16_t Value)
{
    Destination[0] = (uint8_t)Value;
    Destination[1] = (uint8_t)(Value >> 8);
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

static void make_ddc_specific_packet(uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE])
{
    static const uint16_t PairRates[] = {48U, 96U, 192U, 384U};
    uint8_t Index;

    memset(Packet, 0, P2_DDC_SPECIFIC_PACKET_SIZE);
    write_be_u32(Packet, 0x50607080U);
    Packet[4] = P2_SATURN_ADC_COUNT;
    Packet[5] = 0x05U;
    Packet[6] = 0x06U;
    write_le_u16(Packet + 7, 0x0355U);

    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
    {
        const size_t Offset = 17U + ((size_t)Index * 6U);
        const uint16_t SampleRate = (Index < 8U) ? PairRates[Index / 2U] :
                                    ((Index == 8U) ? 768U : 1536U);
        Packet[Offset] = (uint8_t)(Index % eP2DDCSourceCount);
        write_be_u16(Packet + Offset + 1U, SampleRate);
        Packet[Offset + 5U] = 24U;
    }

    Packet[1363] = 0x02U;
    Packet[1365] = 0x08U;
    Packet[1367] = 0x20U;
    Packet[1369] = 0x80U;
}

static void set_ddc_wire_config(uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE],
                                uint8_t DDCIndex, EP2DDCSource Source,
                                uint16_t SampleRate, uint8_t SampleSize)
{
    const size_t Offset = 17U + ((size_t)DDCIndex * 6U);
    Packet[Offset] = (uint8_t)Source;
    write_be_u16(Packet + Offset + 1U, SampleRate);
    Packet[Offset + 5U] = SampleSize;
}

static void mock_set_port(void *Context, EP2GeneralPort Kind, uint8_t Index, uint16_t Port)
{
    TMockGeneralActions *Mock = Context;
    assert(Mock != NULL);
    assert(Kind < eP2GeneralPortCount);
    assert(Index < P2_SATURN_HARDWARE_DDC_COUNT);
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

static void mock_set_adc_count(void *Context, uint8_t Count)
{
    TMockDDCActions *Mock = Context;
    Mock->ActionCount++;
    Mock->ADCCount = Count;
}

static void mock_set_adc_options(void *Context, uint8_t ADCIndex,
                                 bool Dither, bool Random)
{
    TMockDDCActions *Mock = Context;
    assert(ADCIndex < P2_SATURN_ADC_COUNT);
    Mock->ActionCount++;
    Mock->ADCOptionActionCount++;
    Mock->ADCDither[ADCIndex] = Dither;
    Mock->ADCRandom[ADCIndex] = Random;
}

static void mock_set_ddc_config(void *Context, uint8_t DDCIndex,
                                const TP2DDCConfig *Config)
{
    TMockDDCActions *Mock = Context;
    assert(DDCIndex < P2_SATURN_HARDWARE_DDC_COUNT);
    assert(Config != NULL);
    Mock->ActionCount++;
    Mock->DDCActionCount++;
    Mock->DDC[DDCIndex] = *Config;
}

static void mock_commit_ddc_config(void *Context)
{
    TMockDDCActions *Mock = Context;
    Mock->ActionCount++;
    Mock->CommitActionCount++;
}

static const TP2DDCActionSink MockDDCSink = {
    .SetADCCount = mock_set_adc_count,
    .SetADCOptions = mock_set_adc_options,
    .SetDDCConfig = mock_set_ddc_config,
    .CommitDDCConfig = mock_commit_ddc_config,
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
    write_be_u16(Packet + 17, UINT16_MAX - P2_SATURN_HARDWARE_DDC_COUNT + 2U);
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

static void test_ddc_specific_packet_decodes_all_ten_ddcs(void)
{
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE];
    TP2DDCSpecificCommand Command;
    uint8_t Index;

    make_ddc_specific_packet(Packet);
    assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
    assert(Command.Sequence == 0x50607080U);
    assert(Command.ADCCount == P2_SATURN_ADC_COUNT);
    assert(Command.ADCDither[0]);
    assert(!Command.ADCDither[1]);
    assert(!Command.ADCRandom[0]);
    assert(Command.ADCRandom[1]);
    assert(Command.EnableMask == 0x0355U);

    for(Index = 0U; Index < P2_SATURN_HARDWARE_DDC_COUNT; Index++)
    {
        assert(Command.DDC[Index].Enabled);
        assert(Command.DDC[Index].Source == (EP2DDCSource)(Index % eP2DDCSourceCount));
        assert(Command.DDC[Index].SampleSize == 24U);
        assert(Command.DDC[Index].Interleaved ==
               (((Index & 1U) == 0U) && (Index < 8U)));
    }
    assert(Command.DDC[0].SampleRate == 48U);
    assert(Command.DDC[2].SampleRate == 96U);
    assert(Command.DDC[4].SampleRate == 192U);
    assert(Command.DDC[6].SampleRate == 384U);
    assert(Command.DDC[8].SampleRate == 768U);
    assert(Command.DDC[9].SampleRate == 1536U);
}

static void test_ddc_specific_command_applies_through_mock_boundary(void)
{
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE];
    TMockDDCActions Mock = {0};

    make_ddc_specific_packet(Packet);
    assert(P2DecodeAndApplyDDCSpecificCommand(Packet, sizeof(Packet),
                                               &MockDDCSink, &Mock));
    assert(Mock.ActionCount == 14U);
    assert(Mock.ADCOptionActionCount == P2_SATURN_ADC_COUNT);
    assert(Mock.DDCActionCount == P2_SATURN_HARDWARE_DDC_COUNT);
    assert(Mock.CommitActionCount == 1U);
    assert(Mock.ADCCount == P2_SATURN_ADC_COUNT);
    assert(Mock.ADCDither[0] && !Mock.ADCDither[1]);
    assert(!Mock.ADCRandom[0] && Mock.ADCRandom[1]);
    assert(Mock.DDC[0].Enabled && Mock.DDC[0].Interleaved);
    assert(Mock.DDC[1].Enabled && !Mock.DDC[1].Interleaved);
    assert(Mock.DDC[8].Source == eP2DDCSourceTXSamples);
    assert(Mock.DDC[9].SampleRate == 1536U);
}

static void test_sparse_zeus_ddc_shapes_remain_compatible(void)
{
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE];
    TP2DDCSpecificCommand Command;

    memset(Packet, 0, sizeof(Packet));
    write_be_u32(Packet, 7U);
    Packet[4] = P2_SATURN_ADC_COUNT;
    Packet[7] = 0x04U;
    set_ddc_wire_config(Packet, 2U, eP2DDCSourceADC1, 192U, 24U);
    assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
    assert(!Command.DDC[0].Enabled && !Command.DDC[1].Enabled);
    assert(Command.DDC[2].Enabled && Command.DDC[2].SampleRate == 192U);
    assert(!Command.DDC[3].Enabled && Command.DDC[3].SampleSize == 0U);

    Packet[7] = 0x05U;
    set_ddc_wire_config(Packet, 0U, eP2DDCSourceADC1, 192U, 24U);
    set_ddc_wire_config(Packet, 1U, eP2DDCSourceTXSamples, 192U, 24U);
    Packet[1363] = 0x02U;
    assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
    assert(Command.DDC[0].Enabled && Command.DDC[0].Interleaved);
    assert(Command.DDC[1].Enabled && Command.DDC[1].Source == eP2DDCSourceTXSamples);
    assert(Command.DDC[2].Enabled && !Command.DDC[2].Interleaved);
}

static void test_all_legacy_ddc_interleave_combinations_decode(void)
{
    static const size_t SyncOffsets[] = {1363U, 1365U, 1367U, 1369U};
    static const uint8_t SyncMasks[] = {0x02U, 0x08U, 0x20U, 0x80U};
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE];
    TP2DDCSpecificCommand Command;
    uint8_t Combination;
    uint8_t Pair;

    for(Combination = 0U; Combination < 16U; Combination++)
    {
        make_ddc_specific_packet(Packet);
        for(Pair = 0U; Pair < 4U; Pair++)
            Packet[SyncOffsets[Pair]] = ((Combination & (uint8_t)(1U << Pair)) != 0U) ?
                                        SyncMasks[Pair] : 0U;

        assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
        for(Pair = 0U; Pair < 4U; Pair++)
        {
            const uint8_t EvenDDC = (uint8_t)(Pair * 2U);
            const bool Interleaved = (Combination & (uint8_t)(1U << Pair)) != 0U;
            assert(Command.DDC[EvenDDC].Interleaved == Interleaved);
            assert(Command.DDC[EvenDDC + 1U].Enabled == Interleaved);
        }
        assert(Command.DDC[8].Enabled && Command.DDC[9].Enabled);
    }
}

static void test_malformed_ddc_packets_never_reach_actions(void)
{
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE + 1U];
    TP2DDCSpecificCommand Command;
    TMockDDCActions Mock = {0};
    size_t Index;

    make_ddc_specific_packet(Packet);
    assert(!P2DecodeAndApplyDDCSpecificCommand(NULL, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE - 1U,
                                                &MockDDCSink, &Mock));
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE + 1U,
                                                &MockDDCSink, &Mock));

    Packet[4] = 0U;
    memset(&Command, 0xa5, sizeof(Command));
    assert(!P2DecodeDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE, &Command));
    for(Index = 0U; Index < sizeof(Command); Index++)
        assert(((const uint8_t *)&Command)[Index] == 0U);

    make_ddc_specific_packet(Packet);
    Packet[8] |= 0x04U;
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));

    make_ddc_specific_packet(Packet);
    Packet[9] = 0x01U;
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));

    make_ddc_specific_packet(Packet);
    Packet[17U + (9U * 6U)] = eP2DDCSourceCount;
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));

    make_ddc_specific_packet(Packet);
    write_be_u16(Packet + 17U + (8U * 6U) + 1U, 44U);
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));

    make_ddc_specific_packet(Packet);
    Packet[17U + (9U * 6U) + 5U] = 16U;
    assert(!P2DecodeAndApplyDDCSpecificCommand(Packet, P2_DDC_SPECIFIC_PACKET_SIZE,
                                                &MockDDCSink, &Mock));
    assert(Mock.ActionCount == 0U);
}

static void test_incomplete_ddc_sink_and_forged_command_are_rejected(void)
{
    uint8_t Packet[P2_DDC_SPECIFIC_PACKET_SIZE];
    TP2DDCSpecificCommand Command;
    TP2DDCActionSink IncompleteSink = MockDDCSink;
    TMockDDCActions Mock = {0};

    make_ddc_specific_packet(Packet);
    assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
    IncompleteSink.CommitDDCConfig = NULL;
    assert(!P2ApplyDDCSpecificCommand(&Command, &IncompleteSink, &Mock));
    assert(Mock.ActionCount == 0U);

    Command.DDC[3].Interleaved = true;
    assert(!P2ApplyDDCSpecificCommand(&Command, &MockDDCSink, &Mock));
    assert(Mock.ActionCount == 0U);

    assert(P2DecodeDDCSpecificCommand(Packet, sizeof(Packet), &Command));
    Command.DDC[0].Source = (EP2DDCSource)-1;
    assert(!P2ApplyDDCSpecificCommand(&Command, &MockDDCSink, &Mock));
    assert(Mock.ActionCount == 0U);
    assert(!P2ApplyDDCSpecificCommand(NULL, &MockDDCSink, &Mock));
    assert(!P2ApplyDDCSpecificCommand(&Command, NULL, &Mock));
}

int main(void)
{
    test_general_packet_decodes_all_fields();
    test_general_command_applies_through_mock_boundary();
    test_zero_base_ports_preserve_default_port_semantics();
    test_malformed_general_packets_never_reach_actions();
    test_incrementing_port_ranges_cannot_wrap();
    test_incomplete_action_sink_is_rejected_before_first_action();
    test_ddc_specific_packet_decodes_all_ten_ddcs();
    test_ddc_specific_command_applies_through_mock_boundary();
    test_sparse_zeus_ddc_shapes_remain_compatible();
    test_all_legacy_ddc_interleave_combinations_decode();
    test_malformed_ddc_packets_never_reach_actions();
    test_incomplete_ddc_sink_and_forged_command_are_rejected();
    puts("protocol2 command boundary tests passed");
    return 0;
}
