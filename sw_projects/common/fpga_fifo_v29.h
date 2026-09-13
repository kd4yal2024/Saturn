#ifndef FPGA_FIFO_V29_H
#define FPGA_FIFO_V29_H

#include <stdbool.h>
#include <stdint.h>

#define FPGA_FIFO_V29_MIN_FIRMWARE 29U
#define FPGA_FIFO_V29_EXPECTED_BUILD_ID 0x56323900U
#define FPGA_FIFO_V29_CHANNEL_COUNT 4U
#define FPGA_FIFO_V29_SNAPSHOT_POLL_LIMIT 50U

/* V29 FIFO monitor byte addresses in the existing P2 RegisterRead/Write map. */
#define FPGA_FIFO_V29_SNAPSHOT_CONTROL 0x9020U
#define FPGA_FIFO_V29_OCCUPANCY_BASE 0x9024U
#define FPGA_FIFO_V29_MINIMUM_BASE 0x9034U
#define FPGA_FIFO_V29_MAXIMUM_BASE 0x9044U
#define FPGA_FIFO_V29_EVENTS_BASE 0x9054U
#define FPGA_FIFO_V29_BUILD_ID 0x9064U

typedef enum
{
  eFPGAFifoV29DDC = 0,
  eFPGAFifoV29DUC,
  eFPGAFifoV29Microphone,
  eFPGAFifoV29Speaker
} EFPGAFifoV29Channel;

typedef enum
{
  eFPGAFifoV29Unsupported = 0,
  eFPGAFifoV29MarkerMismatch,
  eFPGAFifoV29Available
} EFPGAFifoV29Status;

typedef struct
{
  EFPGAFifoV29Status Status;
  bool Available;
  uint32_t BuildId;
  /* Validity and generation identify only the coherent captured occupancy. */
  bool SnapshotValid;
  uint16_t SnapshotGeneration;
  uint64_t SnapshotTimeoutCount;
  uint32_t OccupancyWords[FPGA_FIFO_V29_CHANNEL_COUNT];
  /* Live FPGA boot-lifetime accumulators, not members of the snapshot. */
  uint32_t MinimumWords[FPGA_FIFO_V29_CHANNEL_COUNT];
  uint32_t MaximumWords[FPGA_FIFO_V29_CHANNEL_COUNT];
  /* Live FPGA boot-lifetime accumulators. Each raw counter combines
   * overflow-signal, full, and empty transitions; it is not a sample-loss
   * counter. */
  uint32_t EventTransitions[FPGA_FIFO_V29_CHANNEL_COUNT];
} TFPGAFifoV29Snapshot;

void FPGAFifoV29Init(uint16_t FirmwareVersion);
bool FPGAFifoV29Sample(void);
bool FPGAFifoV29MaybeSample(void);
void FPGAFifoV29GetSnapshot(TFPGAFifoV29Snapshot *Snapshot);

#endif
