#ifndef FPGA_ADC_V30_H
#define FPGA_ADC_V30_H

#include <stdbool.h>
#include <stdint.h>

#define FPGA_ADC_V30_MIN_FIRMWARE 30U
#define FPGA_ADC_V30_EXPECTED_BUILD_ID 0x56333000U
#define FPGA_ADC_V30_CHANNEL_COUNT 2U
#define FPGA_ADC_V30_SNAPSHOT_RETRY_LIMIT 3U

/* V30 ADC episode byte addresses in the existing P2 RegisterRead map. */
#define FPGA_ADC_V30_SNAPSHOT_STATUS 0x5010U
#define FPGA_ADC_V30_BUILD_ID 0x5020U
#define FPGA_ADC_V30_EPISODE_COUNT_BASE 0x5024U
#define FPGA_ADC_V30_TOTAL_HIGH_CLOCKS_BASE 0x502CU
#define FPGA_ADC_V30_LONGEST_EPISODE_BASE 0x5034U
#define FPGA_ADC_V30_LATEST_EPISODE_BASE 0x503CU
#define FPGA_ADC_V30_LATEST_PEAK_BASE 0x5044U
#define FPGA_ADC_V30_EPISODE_STATE 0x504CU
#define FPGA_ADC_V30_CLOCK_HZ 0x5050U

typedef enum
{
  eFPGAADCV30Unsupported = 0,
  eFPGAADCV30MarkerMismatch,
  eFPGAADCV30Available
} EFPGAADCV30Status;

typedef struct
{
  EFPGAADCV30Status Status;
  bool Available;
  uint32_t BuildId;
  bool SnapshotValid;
  uint16_t SnapshotGeneration;
  uint64_t SnapshotRetryFailureCount;
  uint32_t ClockHz;
  uint32_t EpisodeCount[FPGA_ADC_V30_CHANNEL_COUNT];
  uint32_t TotalHighClocks[FPGA_ADC_V30_CHANNEL_COUNT];
  uint32_t LongestEpisodeClocks[FPGA_ADC_V30_CHANNEL_COUNT];
  uint32_t LatestEpisodeClocks[FPGA_ADC_V30_CHANNEL_COUNT];
  uint32_t LatestEpisodePeak[FPGA_ADC_V30_CHANNEL_COUNT];
  bool EpisodeActive[FPGA_ADC_V30_CHANNEL_COUNT];
  bool EpisodeValid[FPGA_ADC_V30_CHANNEL_COUNT];
} TFPGAADCV30Snapshot;

void FPGAADCV30Init(uint16_t FirmwareVersion);
bool FPGAADCV30Sample(void);
bool FPGAADCV30MaybeSample(void);
void FPGAADCV30GetSnapshot(TFPGAADCV30Snapshot *Snapshot);

#endif
