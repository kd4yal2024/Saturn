#define _DEFAULT_SOURCE

#include "fpga_adc_v30.h"

#include <string.h>
#include <time.h>

#include "hwaccess.h"

#define V30_SNAPSHOT_VALID 0x80000000U

static TFPGAADCV30Snapshot g_snapshot;
static time_t g_last_sample_second;

void FPGAADCV30Init(uint16_t FirmwareVersion)
{
  memset(&g_snapshot, 0, sizeof(g_snapshot));
  g_snapshot.Status = eFPGAADCV30Unsupported;
  g_last_sample_second = 0;

  if (FirmwareVersion < FPGA_ADC_V30_MIN_FIRMWARE)
    return;

  /* Probe only the marker until the V30 register contract is established. */
  g_snapshot.BuildId = RegisterRead(FPGA_ADC_V30_BUILD_ID);
  if (g_snapshot.BuildId != FPGA_ADC_V30_EXPECTED_BUILD_ID)
  {
    g_snapshot.Status = eFPGAADCV30MarkerMismatch;
    return;
  }

  g_snapshot.Status = eFPGAADCV30Available;
  g_snapshot.Available = true;
}

bool FPGAADCV30Sample(void)
{
  unsigned int Attempt;

  if (!g_snapshot.Available)
    return false;

  for (Attempt = 0; Attempt < FPGA_ADC_V30_SNAPSHOT_RETRY_LIMIT; Attempt++)
  {
    TFPGAADCV30Snapshot Candidate = g_snapshot;
    uint32_t BeforeStatus = RegisterRead(FPGA_ADC_V30_SNAPSHOT_STATUS);
    uint32_t State;
    uint32_t AfterStatus;
    unsigned int Channel;

    if ((BeforeStatus & V30_SNAPSHOT_VALID) == 0U)
    {
      g_snapshot.SnapshotValid = false;
      return false;
    }

    for (Channel = 0; Channel < FPGA_ADC_V30_CHANNEL_COUNT; Channel++)
    {
      uint32_t Offset = 4U * Channel;
      Candidate.EpisodeCount[Channel] = RegisterRead(FPGA_ADC_V30_EPISODE_COUNT_BASE + Offset);
      Candidate.TotalHighClocks[Channel] = RegisterRead(FPGA_ADC_V30_TOTAL_HIGH_CLOCKS_BASE + Offset);
      Candidate.LongestEpisodeClocks[Channel] = RegisterRead(FPGA_ADC_V30_LONGEST_EPISODE_BASE + Offset);
      Candidate.LatestEpisodeClocks[Channel] = RegisterRead(FPGA_ADC_V30_LATEST_EPISODE_BASE + Offset);
      Candidate.LatestEpisodePeak[Channel] = RegisterRead(FPGA_ADC_V30_LATEST_PEAK_BASE + Offset);
    }
    State = RegisterRead(FPGA_ADC_V30_EPISODE_STATE);
    Candidate.ClockHz = RegisterRead(FPGA_ADC_V30_CLOCK_HZ);
    AfterStatus = RegisterRead(FPGA_ADC_V30_SNAPSHOT_STATUS);

    if (AfterStatus == BeforeStatus)
    {
      Candidate.SnapshotValid = true;
      Candidate.SnapshotGeneration = (uint16_t)(AfterStatus & 0xFFFFU);
      for (Channel = 0; Channel < FPGA_ADC_V30_CHANNEL_COUNT; Channel++)
      {
        Candidate.EpisodeActive[Channel] = ((State >> Channel) & 1U) != 0U;
        Candidate.EpisodeValid[Channel] = ((State >> (8U + Channel)) & 1U) != 0U;
      }
      g_snapshot = Candidate;
      return true;
    }
  }

  g_snapshot.SnapshotValid = false;
  g_snapshot.SnapshotRetryFailureCount++;
  return false;
}

bool FPGAADCV30MaybeSample(void)
{
  time_t Now = time(NULL);

  if (Now == (time_t)-1)
    return false;
  if ((g_last_sample_second != 0) && (Now == g_last_sample_second))
    return false;

  g_last_sample_second = Now;
  return FPGAADCV30Sample();
}

void FPGAADCV30GetSnapshot(TFPGAADCV30Snapshot *Snapshot)
{
  if (Snapshot != NULL)
    *Snapshot = g_snapshot;
}
