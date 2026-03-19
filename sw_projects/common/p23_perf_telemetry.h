#ifndef P23_PERF_TELEMETRY_H
#define P23_PERF_TELEMETRY_H

#include <stdbool.h>
#include <stdint.h>

#define P23_PERF_MAX_DDC 10U
#define P23_PERF_MAX_PORTS 20U

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
  eP23PerfCounterSpkrPackets,
  eP23PerfCounterSpkrBytes,
  eP23PerfCounterSpkrDMAWrites,
  eP23PerfCounterSpkrDMAWriteBytes,
  eP23PerfCounterSpkrRecvErrors,
  eP23PerfCounterSpkrDMAErrors,
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
void P23PerfTelemetrySetFeatureFlags(bool ControlPanelEnabled, bool GanymedeEnabled,
                                     bool LDGATUEnabled, bool AriesATUEnabled);
void P23PerfTelemetrySetPort(unsigned int PortIndex, uint16_t PortValue);
void P23PerfTelemetrySetDDCConfig(unsigned int DDCIndex, bool Enabled, bool Interleaved,
                                  uint32_t SampleRateKHz);
void P23PerfTelemetrySetWidebandConfig(uint8_t Enables, uint16_t SamplesPerPacket,
                                       uint8_t SampleSizeBits, uint8_t UpdateRateMs,
                                       uint8_t PacketsPerFrame);
void P23PerfTelemetrySetFIFOSnapshot(uint32_t DDCSamples, uint32_t MicSamples,
                                     uint32_t DUCSamples, uint32_t SpeakerSamples,
                                     uint8_t OverflowBits);
void P23PerfTelemetrySetADCSnapshot(uint16_t ADC1Peak, uint16_t ADC2Peak, uint8_t OverflowBits);
void P23PerfTelemetryCounterAdd(EP23PerfCounterId CounterId, uint64_t Delta);
void P23PerfTelemetryMaybeWrite(void);

#endif
