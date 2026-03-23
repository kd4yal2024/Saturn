#include "p23_perf_telemetry.h"

#include <errno.h>
#include <inttypes.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define P23_PERF_TELEMETRY_JSON_FILE "/dev/shm/saturn_p23_perf_stats.json"

typedef struct
{
  bool Enabled;
  bool Interleaved;
  uint32_t SampleRateKHz;
} TP23PerfDDCConfig;

typedef struct
{
  bool SDRActive;
  bool TXMode;
  bool PureSignalEnabled;
  bool ReplyAddressSet;
  bool StartBitReceived;
  bool ThreadError;
  bool ExitRequested;
  bool UseControlPanel;
  bool UseGanymede;
  bool UseLDGATU;
  bool UseAriesATU;
  uint16_t Ports[P23_PERF_MAX_PORTS];
  TP23PerfDDCConfig DDC[P23_PERF_MAX_DDC];
  uint8_t WidebandEnables;
  uint16_t WidebandSamplesPerPacket;
  uint8_t WidebandSampleSizeBits;
  uint8_t WidebandUpdateRateMs;
  uint8_t WidebandPacketsPerFrame;
  uint32_t FIFODDCSamples;
  uint32_t FIFOMicSamples;
  uint32_t FIFODUCSamples;
  uint32_t FIFOSpeakerSamples;
  uint8_t FIFOOverflowBits;
  uint16_t ADC1Peak;
  uint16_t ADC2Peak;
  uint8_t ADCOverflowBits;
  uint32_t DUCQueueFrames;
  uint32_t DUCFIFOFrames;
  uint32_t DUCQueueAgeUs;
  uint8_t DUCWriteMode;
  uint32_t SpeakerUnderQueueFrames;
  uint32_t SpeakerUnderFIFOFrames;
  uint32_t SpeakerUnderQueueAgeUs;
  uint8_t SpeakerUnderMode;
  bool SpeakerUnderGapActive;
} TP23PerfState;

static const char *g_port_names[P23_PERF_MAX_PORTS] =
{
  "command",
  "ddc_specific",
  "duc_specific",
  "high_priority_in",
  "speaker_audio_in",
  "duc_iq_in",
  "high_priority_out",
  "mic_audio_out",
  "ddc_iq_0",
  "ddc_iq_1",
  "ddc_iq_2",
  "ddc_iq_3",
  "ddc_iq_4",
  "ddc_iq_5",
  "ddc_iq_6",
  "ddc_iq_7",
  "ddc_iq_8",
  "ddc_iq_9",
  "wideband_0",
  "wideband_1"
};

static const char *g_counter_names[eP23PerfCounterCount] =
{
  "high_priority_packets",
  "high_priority_bytes",
  "high_priority_send_errors",
  "mic_packets",
  "mic_bytes",
  "mic_dma_reads",
  "mic_dma_read_bytes",
  "mic_send_errors",
  "mic_dma_errors",
  "ddc_packets",
  "ddc_bytes",
  "ddc_dma_reads",
  "ddc_dma_read_bytes",
  "ddc_dma_errors",
  "ddc_partial_sends",
  "ddc_send_errors",
  "ddc_header_errors",
  "wideband_packets",
  "wideband_bytes",
  "wideband_dma_reads",
  "wideband_dma_read_bytes",
  "wideband_send_errors",
  "duc_packets",
  "duc_bytes",
  "duc_dma_writes",
  "duc_dma_write_bytes",
  "duc_recv_errors",
  "duc_dma_errors",
  "duc_gap_events",
  "duc_gap_dropped_frames",
  "duc_queue_drop_events",
  "duc_queue_dropped_frames",
  "speaker_packets",
  "speaker_bytes",
  "speaker_dma_writes",
  "speaker_dma_write_bytes",
  "speaker_recv_errors",
  "speaker_dma_errors",
  "speaker_gap_events",
  "speaker_stall_events",
  "speaker_gap_dropped_frames",
  "speaker_silence_frames",
  "speaker_underrun_queue_empty_events",
  "speaker_underrun_queue_ready_events",
  "fifo_rx_ddc_over_events",
  "fifo_mic_over_events",
  "fifo_duc_under_events",
  "fifo_speaker_under_events",
  "adc_overflow_events"
};

static pthread_mutex_t g_perf_mutex = PTHREAD_MUTEX_INITIALIZER;
static TP23PerfState g_perf_state;
static atomic_ullong g_perf_counters[eP23PerfCounterCount];
static char g_app_name[16] = "unknown";
static uint32_t g_app_version = 0;
static time_t g_started_at = 0;
static time_t g_last_write = 0;

static const char *SpeakerUnderrunModeName(uint8_t Mode)
{
  switch (Mode)
  {
    case 1U:
      return "normal";
    case 2U:
      return "prefill";
    case 3U:
      return "emergency";
    case 4U:
      return "gap_fill";
    default:
      return "unknown";
  }
}

static const char *DUCWriteModeName(uint8_t Mode)
{
  switch (Mode)
  {
    case 1U:
      return "normal";
    case 2U:
      return "prefill";
    case 3U:
      return "emergency";
    default:
      return "unknown";
  }
}

static void AppendCounterJSON(FILE *File)
{
  unsigned int Index;

  fprintf(File, "  \"counters\": {\n");
  for (Index = 0; Index < (unsigned int)eP23PerfCounterCount; Index++)
  {
    fprintf(
      File,
      "    \"%s\": %" PRIu64 "%s\n",
      g_counter_names[Index],
      (uint64_t)atomic_load(&g_perf_counters[Index]),
      (Index + 1U == (unsigned int)eP23PerfCounterCount) ? "" : ","
    );
  }
  fprintf(File, "  }\n");
}

void P23PerfTelemetryInit(const char *AppName, uint32_t AppVersion)
{
  unsigned int Index;

  pthread_mutex_lock(&g_perf_mutex);
  if ((AppName != NULL) && (AppName[0] != '\0'))
  {
    snprintf(g_app_name, sizeof(g_app_name), "%s", AppName);
  }
  else
  {
    snprintf(g_app_name, sizeof(g_app_name), "%s", "unknown");
  }
  g_app_version = AppVersion;
  g_started_at = time(NULL);
  g_last_write = 0;
  memset(&g_perf_state, 0, sizeof(g_perf_state));
  pthread_mutex_unlock(&g_perf_mutex);

  for (Index = 0; Index < (unsigned int)eP23PerfCounterCount; Index++)
  {
    atomic_store(&g_perf_counters[Index], 0U);
  }
}

void P23PerfTelemetrySetRuntimeFlags(bool SDRIsActive, bool TXMode, bool ReplyIsSet,
                                     bool StartBitIsSet, bool ThreadHasError, bool ExitIsRequested)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.SDRActive = SDRIsActive;
  g_perf_state.TXMode = TXMode;
  g_perf_state.ReplyAddressSet = ReplyIsSet;
  g_perf_state.StartBitReceived = StartBitIsSet;
  g_perf_state.ThreadError = ThreadHasError;
  g_perf_state.ExitRequested = ExitIsRequested;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetPureSignalEnabled(bool Enabled)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.PureSignalEnabled = Enabled;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetFeatureFlags(bool ControlPanelEnabled, bool GanymedeEnabled,
                                     bool LDGATUEnabled, bool AriesATUEnabled)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.UseControlPanel = ControlPanelEnabled;
  g_perf_state.UseGanymede = GanymedeEnabled;
  g_perf_state.UseLDGATU = LDGATUEnabled;
  g_perf_state.UseAriesATU = AriesATUEnabled;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetPort(unsigned int PortIndex, uint16_t PortValue)
{
  if (PortIndex >= P23_PERF_MAX_PORTS)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.Ports[PortIndex] = PortValue;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetDDCConfig(unsigned int DDCIndex, bool Enabled, bool Interleaved,
                                  uint32_t SampleRateKHz)
{
  if (DDCIndex >= P23_PERF_MAX_DDC)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.DDC[DDCIndex].Enabled = Enabled;
  g_perf_state.DDC[DDCIndex].Interleaved = Interleaved;
  g_perf_state.DDC[DDCIndex].SampleRateKHz = SampleRateKHz;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetWidebandConfig(uint8_t Enables, uint16_t SamplesPerPacket,
                                       uint8_t SampleSizeBits, uint8_t UpdateRateMs,
                                       uint8_t PacketsPerFrame)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.WidebandEnables = Enables;
  g_perf_state.WidebandSamplesPerPacket = SamplesPerPacket;
  g_perf_state.WidebandSampleSizeBits = SampleSizeBits;
  g_perf_state.WidebandUpdateRateMs = UpdateRateMs;
  g_perf_state.WidebandPacketsPerFrame = PacketsPerFrame;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetFIFOSnapshot(uint32_t DDCSamples, uint32_t MicSamples,
                                     uint32_t DUCSamples, uint32_t SpeakerSamples,
                                     uint8_t OverflowBits)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.FIFODDCSamples = DDCSamples;
  g_perf_state.FIFOMicSamples = MicSamples;
  g_perf_state.FIFODUCSamples = DUCSamples;
  g_perf_state.FIFOSpeakerSamples = SpeakerSamples;
  g_perf_state.FIFOOverflowBits = OverflowBits;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetADCSnapshot(uint16_t ADC1Peak, uint16_t ADC2Peak, uint8_t OverflowBits)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.ADC1Peak = ADC1Peak;
  g_perf_state.ADC2Peak = ADC2Peak;
  g_perf_state.ADCOverflowBits = OverflowBits;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetDUCQueueContext(uint32_t QueueFrames, uint32_t FIFOFrames,
                                        uint32_t QueueAgeUs, uint8_t Mode)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.DUCQueueFrames = QueueFrames;
  g_perf_state.DUCFIFOFrames = FIFOFrames;
  g_perf_state.DUCQueueAgeUs = QueueAgeUs;
  g_perf_state.DUCWriteMode = Mode;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetSpeakerUnderrunContext(uint32_t QueueFrames, uint32_t FIFOFrames,
                                               uint32_t QueueAgeUs, uint8_t Mode,
                                               bool GapActive)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.SpeakerUnderQueueFrames = QueueFrames;
  g_perf_state.SpeakerUnderFIFOFrames = FIFOFrames;
  g_perf_state.SpeakerUnderQueueAgeUs = QueueAgeUs;
  g_perf_state.SpeakerUnderMode = Mode;
  g_perf_state.SpeakerUnderGapActive = GapActive;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetryCounterAdd(EP23PerfCounterId CounterId, uint64_t Delta)
{
  if (CounterId >= eP23PerfCounterCount)
    return;

  atomic_fetch_add(&g_perf_counters[CounterId], Delta);
}

void P23PerfTelemetryMaybeWrite(void)
{
  TP23PerfState Snapshot;
  char TempPath[192];
  FILE *File;
  time_t Now;
  unsigned int Index;
  long long UptimeSeconds = 0;

  Now = time(NULL);
  if (Now == (time_t)-1)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  if ((g_last_write != 0) && (Now == g_last_write))
  {
    pthread_mutex_unlock(&g_perf_mutex);
    return;
  }
  Snapshot = g_perf_state;
  if ((g_started_at != 0) && (Now >= g_started_at))
    UptimeSeconds = (long long)(Now - g_started_at);
  g_last_write = Now;
  pthread_mutex_unlock(&g_perf_mutex);

  snprintf(TempPath, sizeof(TempPath), "%s.%ld.tmp", P23_PERF_TELEMETRY_JSON_FILE, (long)getpid());
  File = fopen(TempPath, "w");
  if (File == NULL)
    return;

  fprintf(File, "{\n");
  fprintf(File, "  \"app\": \"%s\",\n", g_app_name);
  fprintf(File, "  \"version\": %" PRIu32 ",\n", g_app_version);
  fprintf(File, "  \"pid\": %ld,\n", (long)getpid());
  fprintf(File, "  \"timestamp_epoch\": %ld,\n", (long)Now);
  fprintf(File, "  \"uptime_sec\": %lld,\n", UptimeSeconds);
  fprintf(File,
          "  \"state\": {\n"
          "    \"sdr_active\": %s,\n"
          "    \"tx_mode\": %s,\n"
          "    \"pure_signal_enabled\": %s,\n"
          "    \"reply_address_set\": %s,\n"
          "    \"start_bit_received\": %s,\n"
          "    \"thread_error\": %s,\n"
          "    \"exit_requested\": %s\n"
          "  },\n",
          Snapshot.SDRActive ? "true" : "false",
          Snapshot.TXMode ? "true" : "false",
          Snapshot.PureSignalEnabled ? "true" : "false",
          Snapshot.ReplyAddressSet ? "true" : "false",
          Snapshot.StartBitReceived ? "true" : "false",
          Snapshot.ThreadError ? "true" : "false",
          Snapshot.ExitRequested ? "true" : "false");
  fprintf(File,
          "  \"features\": {\n"
          "    \"control_panel\": %s,\n"
          "    \"ganymede\": %s,\n"
          "    \"ldg_atu\": %s,\n"
          "    \"aries_atu\": %s\n"
          "  },\n",
          Snapshot.UseControlPanel ? "true" : "false",
          Snapshot.UseGanymede ? "true" : "false",
          Snapshot.UseLDGATU ? "true" : "false",
          Snapshot.UseAriesATU ? "true" : "false");

  fprintf(File, "  \"routing\": {\n");
  fprintf(File, "    \"ports\": {\n");
  for (Index = 0; Index < P23_PERF_MAX_PORTS; Index++)
  {
    fprintf(File,
            "      \"%s\": %" PRIu16 "%s\n",
            g_port_names[Index],
            Snapshot.Ports[Index],
            (Index + 1U == P23_PERF_MAX_PORTS) ? "" : ",");
  }
  fprintf(File, "    },\n");
  fprintf(File, "    \"ddc\": [\n");
  for (Index = 0; Index < P23_PERF_MAX_DDC; Index++)
  {
    fprintf(File,
            "      { \"id\": %u, \"enabled\": %s, \"interleaved\": %s, \"sample_rate_khz\": %" PRIu32 ", \"port\": %" PRIu16 " }%s\n",
            Index,
            Snapshot.DDC[Index].Enabled ? "true" : "false",
            Snapshot.DDC[Index].Interleaved ? "true" : "false",
            Snapshot.DDC[Index].SampleRateKHz,
            Snapshot.Ports[8U + Index],
            (Index + 1U == P23_PERF_MAX_DDC) ? "" : ",");
  }
  fprintf(File,
          "    ],\n"
          "    \"wideband\": {\n"
          "      \"adc1_enabled\": %s,\n"
          "      \"adc2_enabled\": %s,\n"
          "      \"samples_per_packet\": %" PRIu16 ",\n"
          "      \"sample_size_bits\": %" PRIu8 ",\n"
          "      \"update_rate_ms\": %" PRIu8 ",\n"
          "      \"packets_per_frame\": %" PRIu8 ",\n"
          "      \"port0\": %" PRIu16 ",\n"
          "      \"port1\": %" PRIu16 "\n"
          "    }\n"
          "  },\n",
          (Snapshot.WidebandEnables & 0x01U) ? "true" : "false",
          (Snapshot.WidebandEnables & 0x02U) ? "true" : "false",
          Snapshot.WidebandSamplesPerPacket,
          Snapshot.WidebandSampleSizeBits,
          Snapshot.WidebandUpdateRateMs,
          Snapshot.WidebandPacketsPerFrame,
          Snapshot.Ports[18],
          Snapshot.Ports[19]);

  fprintf(File,
          "  \"gauges\": {\n"
          "    \"fifo_samples\": {\n"
          "      \"ddc\": %" PRIu32 ",\n"
          "      \"mic\": %" PRIu32 ",\n"
          "      \"duc\": %" PRIu32 ",\n"
          "      \"speaker\": %" PRIu32 ",\n"
          "      \"overflow_bits\": %" PRIu8 "\n"
          "    },\n"
          "    \"adc\": {\n"
          "      \"peak1\": %" PRIu16 ",\n"
          "      \"peak2\": %" PRIu16 ",\n"
          "      \"overflow_bits\": %" PRIu8 "\n"
          "    },\n"
          "    \"duc_queue\": {\n"
          "      \"last_queue_frames\": %" PRIu32 ",\n"
          "      \"last_fifo_frames\": %" PRIu32 ",\n"
          "      \"last_queue_age_us\": %" PRIu32 ",\n"
          "      \"last_mode\": \"%s\",\n"
          "      \"last_mode_code\": %" PRIu8 "\n"
          "    },\n"
          "    \"speaker_underrun\": {\n"
          "      \"last_queue_frames\": %" PRIu32 ",\n"
          "      \"last_fifo_frames\": %" PRIu32 ",\n"
          "      \"last_queue_age_us\": %" PRIu32 ",\n"
          "      \"last_mode\": \"%s\",\n"
          "      \"last_mode_code\": %" PRIu8 ",\n"
          "      \"last_gap_active\": %s\n"
          "    }\n"
          "  },\n",
          Snapshot.FIFODDCSamples,
          Snapshot.FIFOMicSamples,
          Snapshot.FIFODUCSamples,
          Snapshot.FIFOSpeakerSamples,
          Snapshot.FIFOOverflowBits,
          Snapshot.ADC1Peak,
          Snapshot.ADC2Peak,
          Snapshot.ADCOverflowBits,
          Snapshot.DUCQueueFrames,
          Snapshot.DUCFIFOFrames,
          Snapshot.DUCQueueAgeUs,
          DUCWriteModeName(Snapshot.DUCWriteMode),
          Snapshot.DUCWriteMode,
          Snapshot.SpeakerUnderQueueFrames,
          Snapshot.SpeakerUnderFIFOFrames,
          Snapshot.SpeakerUnderQueueAgeUs,
          SpeakerUnderrunModeName(Snapshot.SpeakerUnderMode),
          Snapshot.SpeakerUnderMode,
          Snapshot.SpeakerUnderGapActive ? "true" : "false");

  AppendCounterJSON(File);
  fprintf(File, "}\n");

  if (fclose(File) != 0)
  {
    remove(TempPath);
    return;
  }
  if (rename(TempPath, P23_PERF_TELEMETRY_JSON_FILE) != 0)
  {
    remove(TempPath);
    return;
  }
}
