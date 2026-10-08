#ifndef FPGA_RX_COUNTER_V31_H
#define FPGA_RX_COUNTER_V31_H

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#define RXC1_DDC_COUNT 10U
#define RXC1_COUNTER_COUNT 5U

typedef enum
{
  eRXC1Unavailable,
  eRXC1Valid,
  eRXC1Unsupported,
  eRXC1ReadError,
  eRXC1WriteError,
  eRXC1ResetActive,
  eRXC1ResetChanged,
  eRXC1GenerationExhausted,
  eRXC1StaleSnapshot,
  eRXC1StaleSerial,
  eRXC1SnapshotUnavailable,
  eRXC1ReceiverMismatch,
  eRXC1ConfigurationMismatch,
  eRXC1TokenMismatch,
  eRXC1OverflowMismatch,
  eRXC1SnapshotChanged,
  eRXC1AckFailed,
  eRXC1Disabled
} ERXC1Status;

typedef struct
{
  unsigned int Receiver;
  ERXC1Status Status;
  uint32_t SnapshotSerial;
  uint32_t HostToken;
  uint32_t SessionGeneration;
  uint32_t ConfigurationGeneration;
  uint64_t ObservedConfigurationWord;
  uint32_t RateCode;
  uint64_t Counters[RXC1_COUNTER_COUNT];
  bool Overflow;
} TRXC1DDC;

typedef struct
{
  uint64_t SampledAtMs;
  uint64_t HostAcquisitionFailures;
  TRXC1DDC DDC[RXC1_DDC_COUNT];
} TRXC1Snapshot;

void RXC1Init(void);
void RXC1Sample(void);
void RXC1MaybeSample(void);
void RXC1GetSnapshot(TRXC1Snapshot *Snapshot);
void RXC1WriteJSON(FILE *File, const TRXC1Snapshot *Snapshot);
const char *RXC1StatusName(ERXC1Status Status);

#endif
