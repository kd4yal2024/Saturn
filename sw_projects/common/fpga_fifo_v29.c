#define _DEFAULT_SOURCE

#include "fpga_fifo_v29.h"

#include <string.h>
#include <time.h>
#include <unistd.h>

#include "hwaccess.h"

#define V29_SNAPSHOT_REQUEST 0x00000001U
#define V29_SNAPSHOT_VALID 0x80000000U
#define V29_SNAPSHOT_POLL_US 100U

static TFPGAFifoV29Snapshot g_snapshot;
static time_t g_last_sample_second;

void FPGAFifoV29Init(uint16_t FirmwareVersion)
{
  memset(&g_snapshot, 0, sizeof(g_snapshot));
  g_snapshot.Status = eFPGAFifoV29Unsupported;
  g_last_sample_second = 0;

  if (FirmwareVersion < FPGA_FIFO_V29_MIN_FIRMWARE)
    return;

  /* The build marker is the only extended read allowed before support is
   * established. A mismatch must not fan out into the rest of the bank. */
  g_snapshot.BuildId = RegisterRead(FPGA_FIFO_V29_BUILD_ID);
  if (g_snapshot.BuildId != FPGA_FIFO_V29_EXPECTED_BUILD_ID)
  {
    g_snapshot.Status = eFPGAFifoV29MarkerMismatch;
    return;
  }

  g_snapshot.Status = eFPGAFifoV29Available;
  g_snapshot.Available = true;
}

bool FPGAFifoV29Sample(void)
{
  uint32_t BeforeStatus;
  uint16_t BeforeGeneration;
  unsigned int Poll;
  unsigned int Channel;

  if (!g_snapshot.Available)
    return false;

  BeforeStatus = RegisterRead(FPGA_FIFO_V29_SNAPSHOT_CONTROL);
  BeforeGeneration = (uint16_t)(BeforeStatus & 0xFFFFU);
  RegisterWrite(FPGA_FIFO_V29_SNAPSHOT_CONTROL, V29_SNAPSHOT_REQUEST);

  for (Poll = 0; Poll < FPGA_FIFO_V29_SNAPSHOT_POLL_LIMIT; Poll++)
  {
    uint32_t Status = RegisterRead(FPGA_FIFO_V29_SNAPSHOT_CONTROL);
    uint16_t Generation = (uint16_t)(Status & 0xFFFFU);

    if (((Status & V29_SNAPSHOT_VALID) != 0U) && (Generation != BeforeGeneration))
    {
      for (Channel = 0; Channel < FPGA_FIFO_V29_CHANNEL_COUNT; Channel++)
      {
        uint32_t Offset = 4U * Channel;
        g_snapshot.OccupancyWords[Channel] = RegisterRead(FPGA_FIFO_V29_OCCUPANCY_BASE + Offset);
        g_snapshot.MinimumWords[Channel] = RegisterRead(FPGA_FIFO_V29_MINIMUM_BASE + Offset);
        g_snapshot.MaximumWords[Channel] = RegisterRead(FPGA_FIFO_V29_MAXIMUM_BASE + Offset);
        g_snapshot.EventTransitions[Channel] = RegisterRead(FPGA_FIFO_V29_EVENTS_BASE + Offset);
      }
      g_snapshot.SnapshotValid = true;
      g_snapshot.SnapshotGeneration = Generation;
      return true;
    }

    if (Poll + 1U < FPGA_FIFO_V29_SNAPSHOT_POLL_LIMIT)
      usleep(V29_SNAPSHOT_POLL_US);
  }

  g_snapshot.SnapshotValid = false;
  g_snapshot.SnapshotTimeoutCount++;
  return false;
}

bool FPGAFifoV29MaybeSample(void)
{
  time_t Now = time(NULL);

  if (Now == (time_t)-1)
    return false;
  if ((g_last_sample_second != 0) && (Now == g_last_sample_second))
    return false;

  g_last_sample_second = Now;
  return FPGAFifoV29Sample();
}

void FPGAFifoV29GetSnapshot(TFPGAFifoV29Snapshot *Snapshot)
{
  if (Snapshot != NULL)
    *Snapshot = g_snapshot;
}
