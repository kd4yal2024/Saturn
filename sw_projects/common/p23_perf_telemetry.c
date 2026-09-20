#include "p23_perf_telemetry.h"

#include <errno.h>
#include <inttypes.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
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
  bool SpeakerPacingAvailable;
  uint32_t SpeakerThreadTid;
  int32_t SpeakerThreadSchedulingPolicy;
  int32_t SpeakerThreadPriority;
  bool SpeakerLastUnderrunValid;
  uint64_t SpeakerLastUnderrunMonotonicNs;
  uint64_t SpeakerLastUnderrunLoopGapNs;
  uint64_t SpeakerLastUnderrunReceiveDurationNs;
  uint64_t SpeakerLastUnderrunDMAWriteDurationNs;
  uint32_t SpeakerLastUnderrunFIFOFramesBeforeRefill;
  uint32_t SpeakerLastUnderrunQueuedFrames;
  uint32_t SpeakerLastUnderrunFramesSelected;
  uint32_t SpeakerLastUnderrunFramesWritten;
  uint32_t SpeakerLastUnderrunQueueAgeUs;
  int32_t SpeakerLastUnderrunThreadCPU;
  bool FPGAInfoValid;
  TVersionInfoSnapshot FPGAInfo;
  bool DieTempValid;
  float DieTempC;
  TFPGAFifoV29Snapshot FPGAFifoV29;
  TFPGAADCV30Snapshot FPGAADCV30;
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
static atomic_ullong g_speaker_maximum_loop_gap_ns;
static atomic_ullong g_speaker_maximum_receive_duration_ns;
static atomic_ullong g_speaker_maximum_dma_write_duration_ns;
static atomic_ullong g_speaker_loop_gap_over_threshold[P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT];
static atomic_ullong g_speaker_receive_duration_over_threshold[P23_SPEAKER_DURATION_THRESHOLD_COUNT];
static atomic_ullong g_speaker_dma_write_duration_over_threshold[P23_SPEAKER_DURATION_THRESHOLD_COUNT];
static atomic_uint g_speaker_peak_software_queue_frames;
static atomic_int g_speaker_thread_cpu;
static char g_app_name[16] = "unknown";
static uint32_t g_app_version = 0;
static time_t g_started_at = 0;
static time_t g_last_write = 0;

static const uint64_t g_speaker_loop_gap_threshold_ns[P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT] =
{
  2000000ULL, 4000000ULL, 8000000ULL, 16000000ULL
};

static const uint64_t g_speaker_duration_threshold_ns[P23_SPEAKER_DURATION_THRESHOLD_COUNT] =
{
  1000000ULL, 2000000ULL, 4000000ULL, 8000000ULL, 16000000ULL
};

static void AtomicMaximumU64(atomic_ullong *Maximum, uint64_t Candidate)
{
  unsigned long long Observed = atomic_load(Maximum);

  while ((Candidate > Observed) &&
         !atomic_compare_exchange_weak(Maximum, &Observed, Candidate))
  {
  }
}

static void AtomicMaximumU32(atomic_uint *Maximum, uint32_t Candidate)
{
  unsigned int Observed = atomic_load(Maximum);

  while ((Candidate > Observed) &&
         !atomic_compare_exchange_weak(Maximum, &Observed, Candidate))
  {
  }
}

static void ObserveSpeakerDuration(uint64_t DurationNs, atomic_ullong *Maximum,
                                   const uint64_t *Thresholds, atomic_ullong *Counters,
                                   unsigned int ThresholdCount)
{
  unsigned int Index;

  AtomicMaximumU64(Maximum, DurationNs);
  for (Index = 0; Index < ThresholdCount; Index++)
  {
    if (DurationNs > Thresholds[Index])
      atomic_fetch_add(&Counters[Index], 1U);
  }
}

static const char *SchedulingPolicyName(int32_t Policy)
{
  switch (Policy)
  {
    case SCHED_OTHER:
      return "other";
    case SCHED_FIFO:
      return "fifo";
    case SCHED_RR:
      return "round_robin";
#ifdef SCHED_BATCH
    case SCHED_BATCH:
      return "batch";
#endif
#ifdef SCHED_IDLE
    case SCHED_IDLE:
      return "idle";
#endif
#ifdef SCHED_DEADLINE
    case SCHED_DEADLINE:
      return "deadline";
#endif
    default:
      return "unknown";
  }
}

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

static const char *FPGAFifoV29StatusJSON(EFPGAFifoV29Status Status)
{
  switch (Status)
  {
    case eFPGAFifoV29Available:
      return "available";
    case eFPGAFifoV29MarkerMismatch:
      return "marker_mismatch";
    default:
      return "unsupported";
  }
}

static const char *FPGAADCV30StatusJSON(EFPGAADCV30Status Status)
{
  switch (Status)
  {
    case eFPGAADCV30Available:
      return "available";
    case eFPGAADCV30MarkerMismatch:
      return "marker_mismatch";
    default:
      return "unsupported";
  }
}

void P23PerfTelemetryWriteFPGAADCV30JSON(FILE *File, const TFPGAADCV30Snapshot *Snapshot)
{
  if ((File == NULL) || (Snapshot == NULL))
    return;

  fprintf(File,
          "    \"fpga_adc_v30\": {\n"
          "      \"available\": %s,\n"
          "      \"status\": \"%s\",\n"
          "      \"build_id\": %" PRIu32 ",\n"
          "      \"snapshot_valid\": %s,\n"
          "      \"snapshot_generation\": %" PRIu16 ",\n"
          "      \"snapshot_retry_failure_count\": %" PRIu64 ",\n"
          "      \"lifetime_scope\": \"fpga_boot\",\n"
          "      \"duration_unit\": \"adc_clocks\",\n"
          "      \"clock_hz\": %" PRIu32 ",\n"
          "      \"adc1\": { \"episode_count\": %" PRIu32 ", \"total_high_clocks\": %" PRIu32 ", \"longest_episode_clocks\": %" PRIu32 ", \"latest_episode_clocks\": %" PRIu32 ", \"latest_episode_peak\": %" PRIu32 ", \"episode_active\": %s, \"episode_valid\": %s },\n"
          "      \"adc2\": { \"episode_count\": %" PRIu32 ", \"total_high_clocks\": %" PRIu32 ", \"longest_episode_clocks\": %" PRIu32 ", \"latest_episode_clocks\": %" PRIu32 ", \"latest_episode_peak\": %" PRIu32 ", \"episode_active\": %s, \"episode_valid\": %s }\n"
          "    },\n",
          Snapshot->Available ? "true" : "false",
          FPGAADCV30StatusJSON(Snapshot->Status),
          Snapshot->BuildId,
          Snapshot->SnapshotValid ? "true" : "false",
          Snapshot->SnapshotGeneration,
          Snapshot->SnapshotRetryFailureCount,
          Snapshot->ClockHz,
          Snapshot->EpisodeCount[0], Snapshot->TotalHighClocks[0],
          Snapshot->LongestEpisodeClocks[0], Snapshot->LatestEpisodeClocks[0],
          Snapshot->LatestEpisodePeak[0],
          Snapshot->EpisodeActive[0] ? "true" : "false",
          Snapshot->EpisodeValid[0] ? "true" : "false",
          Snapshot->EpisodeCount[1], Snapshot->TotalHighClocks[1],
          Snapshot->LongestEpisodeClocks[1], Snapshot->LatestEpisodeClocks[1],
          Snapshot->LatestEpisodePeak[1],
          Snapshot->EpisodeActive[1] ? "true" : "false",
          Snapshot->EpisodeValid[1] ? "true" : "false");
}

void P23PerfTelemetryWriteFPGAFifoV29JSON(FILE *File, const TFPGAFifoV29Snapshot *Snapshot)
{
  static const char *Names[FPGA_FIFO_V29_CHANNEL_COUNT] = {"ddc", "duc", "mic", "speaker"};
  const uint32_t *Groups[4];
  static const char *GroupNames[4] = {
    "occupancy_words", "minimum_words", "maximum_words", "event_transitions"
  };
  unsigned int Group;
  unsigned int Channel;

  if ((File == NULL) || (Snapshot == NULL))
    return;

  Groups[0] = Snapshot->OccupancyWords;
  Groups[1] = Snapshot->MinimumWords;
  Groups[2] = Snapshot->MaximumWords;
  Groups[3] = Snapshot->EventTransitions;

  fprintf(File,
          "    \"fpga_fifo_v29\": {\n"
          "      \"available\": %s,\n"
          "      \"status\": \"%s\",\n"
          "      \"build_id\": %" PRIu32 ",\n"
          "      \"snapshot_valid\": %s,\n"
          "      \"snapshot_generation\": %" PRIu16 ",\n"
          "      \"snapshot_timeout_count\": %" PRIu64 ",\n",
          Snapshot->Available ? "true" : "false",
          FPGAFifoV29StatusJSON(Snapshot->Status),
          Snapshot->BuildId,
          Snapshot->SnapshotValid ? "true" : "false",
          Snapshot->SnapshotGeneration,
          Snapshot->SnapshotTimeoutCount);

  for (Group = 0; Group < 4U; Group++)
  {
    fprintf(File, "      \"%s\": {\n", GroupNames[Group]);
    for (Channel = 0; Channel < FPGA_FIFO_V29_CHANNEL_COUNT; Channel++)
    {
      fprintf(File, "        \"%s\": %" PRIu32 "%s\n",
              Names[Channel], Groups[Group][Channel],
              (Channel + 1U == FPGA_FIFO_V29_CHANNEL_COUNT) ? "" : ",");
    }
    fprintf(File, "      }%s\n", (Group == 3U) ? "" : ",");
  }
  fprintf(File, "    }\n");
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
  atomic_store(&g_speaker_maximum_loop_gap_ns, 0U);
  atomic_store(&g_speaker_maximum_receive_duration_ns, 0U);
  atomic_store(&g_speaker_maximum_dma_write_duration_ns, 0U);
  atomic_store(&g_speaker_peak_software_queue_frames, 0U);
  atomic_store(&g_speaker_thread_cpu, -1);
  for (Index = 0; Index < P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT; Index++)
    atomic_store(&g_speaker_loop_gap_over_threshold[Index], 0U);
  for (Index = 0; Index < P23_SPEAKER_DURATION_THRESHOLD_COUNT; Index++)
  {
    atomic_store(&g_speaker_receive_duration_over_threshold[Index], 0U);
    atomic_store(&g_speaker_dma_write_duration_over_threshold[Index], 0U);
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

void P23PerfTelemetrySetVersionInfo(const TVersionInfoSnapshot *Snapshot)
{
  if (Snapshot == NULL)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.FPGAInfo = *Snapshot;
  g_perf_state.FPGAInfoValid = true;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetDieTempC(float TempC)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.DieTempC = TempC;
  g_perf_state.DieTempValid = true;
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

void P23PerfTelemetrySetFPGAFifoV29(const TFPGAFifoV29Snapshot *Snapshot)
{
  if (Snapshot == NULL)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.FPGAFifoV29 = *Snapshot;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetFPGAADCV30(const TFPGAADCV30Snapshot *Snapshot)
{
  if (Snapshot == NULL)
    return;

  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.FPGAADCV30 = *Snapshot;
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

void P23PerfTelemetrySetSpeakerThread(uint32_t Tid, int32_t SchedulingPolicy,
                                      int32_t Priority, int32_t CPU)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.SpeakerPacingAvailable = true;
  g_perf_state.SpeakerThreadTid = Tid;
  g_perf_state.SpeakerThreadSchedulingPolicy = SchedulingPolicy;
  g_perf_state.SpeakerThreadPriority = Priority;
  pthread_mutex_unlock(&g_perf_mutex);
  atomic_store(&g_speaker_thread_cpu, CPU);
}

void P23PerfTelemetrySetSpeakerThreadCPU(int32_t CPU)
{
  atomic_store(&g_speaker_thread_cpu, CPU);
}

void P23PerfTelemetryObserveSpeakerLoopGap(uint64_t DurationNs)
{
  ObserveSpeakerDuration(DurationNs, &g_speaker_maximum_loop_gap_ns,
                         g_speaker_loop_gap_threshold_ns,
                         g_speaker_loop_gap_over_threshold,
                         P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT);
}

void P23PerfTelemetryObserveSpeakerReceiveDuration(uint64_t DurationNs)
{
  ObserveSpeakerDuration(DurationNs, &g_speaker_maximum_receive_duration_ns,
                         g_speaker_duration_threshold_ns,
                         g_speaker_receive_duration_over_threshold,
                         P23_SPEAKER_DURATION_THRESHOLD_COUNT);
}

void P23PerfTelemetryObserveSpeakerDMAWriteDuration(uint64_t DurationNs)
{
  ObserveSpeakerDuration(DurationNs, &g_speaker_maximum_dma_write_duration_ns,
                         g_speaker_duration_threshold_ns,
                         g_speaker_dma_write_duration_over_threshold,
                         P23_SPEAKER_DURATION_THRESHOLD_COUNT);
}

void P23PerfTelemetryObserveSpeakerQueueDepth(uint32_t QueueFrames)
{
  AtomicMaximumU32(&g_speaker_peak_software_queue_frames, QueueFrames);
}

void P23PerfTelemetrySetSpeakerUnderrunContextWithPacing(
  uint32_t QueueFrames, uint32_t FIFOFrames, uint32_t QueueAgeUs, uint8_t Mode,
  bool GapActive, uint64_t LoopGapNs, uint64_t ReceiveDurationNs,
  uint64_t DMAWriteDurationNs, uint64_t EventMonotonicNs, int32_t ThreadCPU)
{
  pthread_mutex_lock(&g_perf_mutex);
  g_perf_state.SpeakerUnderQueueFrames = QueueFrames;
  g_perf_state.SpeakerUnderFIFOFrames = FIFOFrames;
  g_perf_state.SpeakerUnderQueueAgeUs = QueueAgeUs;
  g_perf_state.SpeakerUnderMode = Mode;
  g_perf_state.SpeakerUnderGapActive = GapActive;
  g_perf_state.SpeakerLastUnderrunValid = true;
  g_perf_state.SpeakerLastUnderrunMonotonicNs = EventMonotonicNs;
  g_perf_state.SpeakerLastUnderrunLoopGapNs = LoopGapNs;
  g_perf_state.SpeakerLastUnderrunReceiveDurationNs = ReceiveDurationNs;
  g_perf_state.SpeakerLastUnderrunDMAWriteDurationNs = DMAWriteDurationNs;
  g_perf_state.SpeakerLastUnderrunFIFOFramesBeforeRefill = FIFOFrames;
  g_perf_state.SpeakerLastUnderrunQueuedFrames = QueueFrames;
  g_perf_state.SpeakerLastUnderrunFramesSelected = 0U;
  g_perf_state.SpeakerLastUnderrunFramesWritten = 0U;
  g_perf_state.SpeakerLastUnderrunQueueAgeUs = QueueAgeUs;
  g_perf_state.SpeakerLastUnderrunThreadCPU = ThreadCPU;
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetrySetSpeakerUnderrunRefill(uint64_t EventMonotonicNs,
                                              uint32_t FramesSelected,
                                              uint32_t FramesWritten)
{
  pthread_mutex_lock(&g_perf_mutex);
  if (g_perf_state.SpeakerLastUnderrunValid &&
      (g_perf_state.SpeakerLastUnderrunMonotonicNs == EventMonotonicNs))
  {
    g_perf_state.SpeakerLastUnderrunFramesSelected = FramesSelected;
    g_perf_state.SpeakerLastUnderrunFramesWritten = FramesWritten;
  }
  pthread_mutex_unlock(&g_perf_mutex);
}

void P23PerfTelemetryGetSpeakerPacingDiagnostics(TP23SpeakerPacingDiagnostics *Snapshot)
{
  unsigned int Index;

  if (Snapshot == NULL)
    return;

  memset(Snapshot, 0, sizeof(*Snapshot));
  pthread_mutex_lock(&g_perf_mutex);
  Snapshot->Available = g_perf_state.SpeakerPacingAvailable;
  Snapshot->ThreadTid = g_perf_state.SpeakerThreadTid;
  Snapshot->ThreadSchedulingPolicy = g_perf_state.SpeakerThreadSchedulingPolicy;
  Snapshot->ThreadPriority = g_perf_state.SpeakerThreadPriority;
  Snapshot->LastUnderrunValid = g_perf_state.SpeakerLastUnderrunValid;
  Snapshot->LastUnderrunMonotonicNs = g_perf_state.SpeakerLastUnderrunMonotonicNs;
  Snapshot->LastUnderrunLoopGapNs = g_perf_state.SpeakerLastUnderrunLoopGapNs;
  Snapshot->LastUnderrunReceiveDurationNs = g_perf_state.SpeakerLastUnderrunReceiveDurationNs;
  Snapshot->LastUnderrunDMAWriteDurationNs = g_perf_state.SpeakerLastUnderrunDMAWriteDurationNs;
  Snapshot->LastUnderrunFIFOFramesBeforeRefill = g_perf_state.SpeakerLastUnderrunFIFOFramesBeforeRefill;
  Snapshot->LastUnderrunQueuedFrames = g_perf_state.SpeakerLastUnderrunQueuedFrames;
  Snapshot->LastUnderrunFramesSelected = g_perf_state.SpeakerLastUnderrunFramesSelected;
  Snapshot->LastUnderrunFramesWritten = g_perf_state.SpeakerLastUnderrunFramesWritten;
  Snapshot->LastUnderrunQueueAgeUs = g_perf_state.SpeakerLastUnderrunQueueAgeUs;
  Snapshot->LastUnderrunThreadCPU = g_perf_state.SpeakerLastUnderrunThreadCPU;
  pthread_mutex_unlock(&g_perf_mutex);

  Snapshot->ThreadCPU = atomic_load(&g_speaker_thread_cpu);
  Snapshot->MaximumLoopGapNs = atomic_load(&g_speaker_maximum_loop_gap_ns);
  Snapshot->MaximumReceiveDurationNs = atomic_load(&g_speaker_maximum_receive_duration_ns);
  Snapshot->MaximumDMAWriteDurationNs = atomic_load(&g_speaker_maximum_dma_write_duration_ns);
  Snapshot->PeakSoftwareQueueFrames = atomic_load(&g_speaker_peak_software_queue_frames);
  for (Index = 0; Index < P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT; Index++)
    Snapshot->LoopGapOverThreshold[Index] = atomic_load(&g_speaker_loop_gap_over_threshold[Index]);
  for (Index = 0; Index < P23_SPEAKER_DURATION_THRESHOLD_COUNT; Index++)
  {
    Snapshot->ReceiveDurationOverThreshold[Index] = atomic_load(&g_speaker_receive_duration_over_threshold[Index]);
    Snapshot->DMAWriteDurationOverThreshold[Index] = atomic_load(&g_speaker_dma_write_duration_over_threshold[Index]);
  }
}

void P23PerfTelemetryWriteSpeakerPacingJSON(FILE *File,
                                            const TP23SpeakerPacingDiagnostics *Snapshot)
{
  if ((File == NULL) || (Snapshot == NULL))
    return;

  fprintf(File,
          "    \"speaker_pacing_diagnostics\": {\n"
          "      \"available\": %s,\n"
          "      \"lifetime_scope\": \"process\",\n"
          "      \"duration_unit\": \"microseconds\",\n"
          "      \"thread\": { \"tid\": %" PRIu32 ", \"scheduling_policy\": \"%s\", \"scheduling_policy_code\": %" PRIi32 ", \"priority\": %" PRIi32 ", \"cpu\": %" PRIi32 " },\n"
          "      \"maximum\": { \"loop_gap_us\": %" PRIu64 ", \"recvmmsg_duration_us\": %" PRIu64 ", \"dma_write_duration_us\": %" PRIu64 ", \"software_queue_depth_frames\": %" PRIu32 " },\n"
          "      \"loop_gap_counts\": { \"over_2ms\": %" PRIu64 ", \"over_4ms\": %" PRIu64 ", \"over_8ms\": %" PRIu64 ", \"over_16ms\": %" PRIu64 " },\n"
          "      \"recvmmsg_duration_counts\": { \"over_1ms\": %" PRIu64 ", \"over_2ms\": %" PRIu64 ", \"over_4ms\": %" PRIu64 ", \"over_8ms\": %" PRIu64 ", \"over_16ms\": %" PRIu64 " },\n"
          "      \"dma_write_duration_counts\": { \"over_1ms\": %" PRIu64 ", \"over_2ms\": %" PRIu64 ", \"over_4ms\": %" PRIu64 ", \"over_8ms\": %" PRIu64 ", \"over_16ms\": %" PRIu64 " },\n"
          "      \"last_underrun\": { \"valid\": %s, \"monotonic_timestamp_ns\": %" PRIu64 ", \"loop_gap_us\": %" PRIu64 ", \"recvmmsg_duration_us\": %" PRIu64 ", \"dma_write_duration_us\": %" PRIu64 ", \"fifo_frames_before_refill\": %" PRIu32 ", \"queued_frames\": %" PRIu32 ", \"frames_selected\": %" PRIu32 ", \"frames_written\": %" PRIu32 ", \"queue_age_us\": %" PRIu32 ", \"thread_cpu\": %" PRIi32 " }\n"
          "    },\n",
          Snapshot->Available ? "true" : "false",
          Snapshot->ThreadTid,
          SchedulingPolicyName(Snapshot->ThreadSchedulingPolicy),
          Snapshot->ThreadSchedulingPolicy,
          Snapshot->ThreadPriority,
          Snapshot->ThreadCPU,
          (uint64_t)(Snapshot->MaximumLoopGapNs / 1000U),
          (uint64_t)(Snapshot->MaximumReceiveDurationNs / 1000U),
          (uint64_t)(Snapshot->MaximumDMAWriteDurationNs / 1000U),
          Snapshot->PeakSoftwareQueueFrames,
          Snapshot->LoopGapOverThreshold[0], Snapshot->LoopGapOverThreshold[1],
          Snapshot->LoopGapOverThreshold[2], Snapshot->LoopGapOverThreshold[3],
          Snapshot->ReceiveDurationOverThreshold[0], Snapshot->ReceiveDurationOverThreshold[1],
          Snapshot->ReceiveDurationOverThreshold[2], Snapshot->ReceiveDurationOverThreshold[3],
          Snapshot->ReceiveDurationOverThreshold[4],
          Snapshot->DMAWriteDurationOverThreshold[0], Snapshot->DMAWriteDurationOverThreshold[1],
          Snapshot->DMAWriteDurationOverThreshold[2], Snapshot->DMAWriteDurationOverThreshold[3],
          Snapshot->DMAWriteDurationOverThreshold[4],
          Snapshot->LastUnderrunValid ? "true" : "false",
          Snapshot->LastUnderrunMonotonicNs,
          (uint64_t)(Snapshot->LastUnderrunLoopGapNs / 1000U),
          (uint64_t)(Snapshot->LastUnderrunReceiveDurationNs / 1000U),
          (uint64_t)(Snapshot->LastUnderrunDMAWriteDurationNs / 1000U),
          Snapshot->LastUnderrunFIFOFramesBeforeRefill,
          Snapshot->LastUnderrunQueuedFrames,
          Snapshot->LastUnderrunFramesSelected,
          Snapshot->LastUnderrunFramesWritten,
          Snapshot->LastUnderrunQueueAgeUs,
          Snapshot->LastUnderrunThreadCPU);
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
  TP23SpeakerPacingDiagnostics SpeakerPacingSnapshot;
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
  P23PerfTelemetryGetSpeakerPacingDiagnostics(&SpeakerPacingSnapshot);

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
  fprintf(File,
          "  \"fpga\": {\n"
          "    \"available\": %s,\n"
          "    \"product_id\": %" PRIu16 ",\n"
          "    \"product\": \"%s\",\n"
          "    \"product_version\": %" PRIu16 ",\n"
          "    \"firmware_id\": %" PRIu8 ",\n"
          "    \"firmware_name\": \"%s\",\n"
          "    \"firmware_version\": %" PRIu16 ",\n"
          "    \"firmware_major_version\": %" PRIu8 ",\n"
          "    \"date_code_raw\": %" PRIu32 ",\n"
          "    \"date_code_hex\": \"%08" PRIX32 "\",\n"
          "    \"clock_mask\": %" PRIu8 ",\n"
          "    \"all_clocks_present\": %s,\n"
          "    \"fallback_config\": %s,\n"
          "    \"die_temp_valid\": %s,\n"
          "    \"die_temp_c\": %.1f\n"
          "  },\n",
          Snapshot.FPGAInfoValid ? "true" : "false",
          Snapshot.FPGAInfo.ProductId,
          Snapshot.FPGAInfo.ProductName,
          Snapshot.FPGAInfo.ProductVersion,
          Snapshot.FPGAInfo.FirmwareId,
          Snapshot.FPGAInfo.FirmwareName,
          Snapshot.FPGAInfo.FirmwareVersion,
          Snapshot.FPGAInfo.FirmwareMajorVersion,
          Snapshot.FPGAInfo.DateCode,
          Snapshot.FPGAInfo.DateCode,
          Snapshot.FPGAInfo.ClockMask,
          Snapshot.FPGAInfo.AllClocksPresent ? "true" : "false",
          Snapshot.FPGAInfo.FallbackConfig ? "true" : "false",
          Snapshot.DieTempValid ? "true" : "false",
          Snapshot.DieTempC);

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
          "    },\n",
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

  P23PerfTelemetryWriteSpeakerPacingJSON(File, &SpeakerPacingSnapshot);
  P23PerfTelemetryWriteFPGAADCV30JSON(File, &Snapshot.FPGAADCV30);
  P23PerfTelemetryWriteFPGAFifoV29JSON(File, &Snapshot.FPGAFifoV29);
  fprintf(File, "  },\n");

  AppendCounterJSON(File);
  fprintf(File, "}\n");

  if (fclose(File) != 0)
  {
    remove(TempPath);
    return;
  }
  /*
   * p2app runs under the dedicated saturn-radio account with UMask=0027,
   * while Saturn Go reads this non-sensitive snapshot as its own service
   * user. Set the publication mode explicitly before the atomic rename so
   * the dashboard can consume the counters without weakening the service
   * unit's default umask.
   */
  if (chmod(TempPath, 0644) != 0)
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
