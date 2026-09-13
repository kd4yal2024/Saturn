#ifndef P23_PERF_TELEMETRY_H
#define P23_PERF_TELEMETRY_H

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#include "fpga_fifo_v29.h"
#include "version.h"

#define P23_PERF_MAX_DDC 10U
#define P23_PERF_MAX_PORTS 20U
#define P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT 4U
#define P23_SPEAKER_DURATION_THRESHOLD_COUNT 5U

typedef struct
{
  bool Available;
  uint32_t ThreadTid;
  int32_t ThreadSchedulingPolicy;
  int32_t ThreadPriority;
  int32_t ThreadCPU;
  uint64_t MaximumLoopGapNs;
  uint64_t MaximumReceiveDurationNs;
  uint64_t MaximumDMAWriteDurationNs;
  uint64_t LoopGapOverThreshold[P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT];
  uint64_t ReceiveDurationOverThreshold[P23_SPEAKER_DURATION_THRESHOLD_COUNT];
  uint64_t DMAWriteDurationOverThreshold[P23_SPEAKER_DURATION_THRESHOLD_COUNT];
  uint32_t PeakSoftwareQueueFrames;
  bool LastUnderrunValid;
  uint64_t LastUnderrunMonotonicNs;
  uint64_t LastUnderrunLoopGapNs;
  uint64_t LastUnderrunReceiveDurationNs;
  uint64_t LastUnderrunDMAWriteDurationNs;
  uint32_t LastUnderrunFIFOFramesBeforeRefill;
  uint32_t LastUnderrunQueuedFrames;
  uint32_t LastUnderrunFramesSelected;
  uint32_t LastUnderrunFramesWritten;
  uint32_t LastUnderrunQueueAgeUs;
  int32_t LastUnderrunThreadCPU;
} TP23SpeakerPacingDiagnostics;

typedef enum
{
  eP23PerfCounterHighPriorityPackets = 0,
  eP23PerfCounterHighPriorityBytes,
  eP23PerfCounterHighPrioritySendErrors,
  eP23PerfCounterMicPackets,
  eP23PerfCounterMicBytes,
  eP23PerfCounterMicDMAReads,
  eP23PerfCounterMicDMAReadBytes,
  eP23PerfCounterMicSendErrors,
  eP23PerfCounterMicDMAErrors,
  eP23PerfCounterDDCPackets,
  eP23PerfCounterDDCBytes,
  eP23PerfCounterDDCDMAReads,
  eP23PerfCounterDDCDMAReadBytes,
  eP23PerfCounterDDCDMAErrors,
  eP23PerfCounterDDCPartialSends,
  eP23PerfCounterDDCSendErrors,
  eP23PerfCounterDDCHeaderErrors,
  eP23PerfCounterWidebandPackets,
  eP23PerfCounterWidebandBytes,
  eP23PerfCounterWidebandDMAReads,
  eP23PerfCounterWidebandDMAReadBytes,
  eP23PerfCounterWidebandSendErrors,
  eP23PerfCounterDUCPackets,
  eP23PerfCounterDUCBytes,
  eP23PerfCounterDUCDMAWrites,
  eP23PerfCounterDUCDMAWriteBytes,
  eP23PerfCounterDUCRecvErrors,
  eP23PerfCounterDUCDMAErrors,
  eP23PerfCounterDUCGapEvents,
  eP23PerfCounterDUCGapDroppedFrames,
  eP23PerfCounterDUCQueueDropEvents,
  eP23PerfCounterDUCQueueDroppedFrames,
  eP23PerfCounterSpkrPackets,
  eP23PerfCounterSpkrBytes,
  eP23PerfCounterSpkrDMAWrites,
  eP23PerfCounterSpkrDMAWriteBytes,
  eP23PerfCounterSpkrRecvErrors,
  eP23PerfCounterSpkrDMAErrors,
  eP23PerfCounterSpkrGapEvents,
  eP23PerfCounterSpkrStallEvents,
  eP23PerfCounterSpkrGapDroppedFrames,
  eP23PerfCounterSpkrSilenceFrames,
  eP23PerfCounterSpkrUnderQueueEmpty,
  eP23PerfCounterSpkrUnderQueueReady,
  eP23PerfCounterFIFORXDdcOver,
  eP23PerfCounterFIFOMicOver,
  eP23PerfCounterFIFODucUnder,
  eP23PerfCounterFIFOSpkrUnder,
  eP23PerfCounterADCOverflowEvents,
  eP23PerfCounterCount
} EP23PerfCounterId;

void P23PerfTelemetryInit(const char *AppName, uint32_t AppVersion);
void P23PerfTelemetrySetRuntimeFlags(bool SDRIsActive, bool TXMode, bool ReplyIsSet,
                                     bool StartBitIsSet, bool ThreadHasError, bool ExitIsRequested);
void P23PerfTelemetrySetPureSignalEnabled(bool Enabled);
void P23PerfTelemetrySetFeatureFlags(bool ControlPanelEnabled, bool GanymedeEnabled,
                                     bool LDGATUEnabled, bool AriesATUEnabled);
void P23PerfTelemetrySetVersionInfo(const TVersionInfoSnapshot *Snapshot);
void P23PerfTelemetrySetDieTempC(float TempC);
void P23PerfTelemetrySetPort(unsigned int PortIndex, uint16_t PortValue);
void P23PerfTelemetrySetDDCConfig(unsigned int DDCIndex, bool Enabled, bool Interleaved,
                                  uint32_t SampleRateKHz);
void P23PerfTelemetrySetWidebandConfig(uint8_t Enables, uint16_t SamplesPerPacket,
                                       uint8_t SampleSizeBits, uint8_t UpdateRateMs,
                                       uint8_t PacketsPerFrame);
void P23PerfTelemetrySetFIFOSnapshot(uint32_t DDCSamples, uint32_t MicSamples,
                                     uint32_t DUCSamples, uint32_t SpeakerSamples,
                                     uint8_t OverflowBits);
void P23PerfTelemetrySetFPGAFifoV29(const TFPGAFifoV29Snapshot *Snapshot);
void P23PerfTelemetryWriteFPGAFifoV29JSON(FILE *File, const TFPGAFifoV29Snapshot *Snapshot);
void P23PerfTelemetrySetADCSnapshot(uint16_t ADC1Peak, uint16_t ADC2Peak, uint8_t OverflowBits);
void P23PerfTelemetrySetDUCQueueContext(uint32_t QueueFrames, uint32_t FIFOFrames,
                                        uint32_t QueueAgeUs, uint8_t Mode);
void P23PerfTelemetrySetSpeakerUnderrunContext(uint32_t QueueFrames, uint32_t FIFOFrames,
                                               uint32_t QueueAgeUs, uint8_t Mode,
                                               bool GapActive);
void P23PerfTelemetrySetSpeakerThread(uint32_t Tid, int32_t SchedulingPolicy,
                                      int32_t Priority, int32_t CPU);
void P23PerfTelemetrySetSpeakerThreadCPU(int32_t CPU);
void P23PerfTelemetryObserveSpeakerLoopGap(uint64_t DurationNs);
void P23PerfTelemetryObserveSpeakerReceiveDuration(uint64_t DurationNs);
void P23PerfTelemetryObserveSpeakerDMAWriteDuration(uint64_t DurationNs);
void P23PerfTelemetryObserveSpeakerQueueDepth(uint32_t QueueFrames);
void P23PerfTelemetrySetSpeakerUnderrunContextWithPacing(
  uint32_t QueueFrames, uint32_t FIFOFrames, uint32_t QueueAgeUs, uint8_t Mode,
  bool GapActive, uint64_t LoopGapNs, uint64_t ReceiveDurationNs,
  uint64_t DMAWriteDurationNs, uint64_t EventMonotonicNs, int32_t ThreadCPU);
void P23PerfTelemetrySetSpeakerUnderrunRefill(uint64_t EventMonotonicNs,
                                              uint32_t FramesSelected,
                                              uint32_t FramesWritten);
void P23PerfTelemetryGetSpeakerPacingDiagnostics(TP23SpeakerPacingDiagnostics *Snapshot);
void P23PerfTelemetryWriteSpeakerPacingJSON(FILE *File,
                                            const TP23SpeakerPacingDiagnostics *Snapshot);
void P23PerfTelemetryCounterAdd(EP23PerfCounterId CounterId, uint64_t Delta);
void P23PerfTelemetryMaybeWrite(void);

#endif
