#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../../common/fpga_adc_v30.h"
#include "../../common/p23_perf_telemetry.h"

static uint32_t marker = FPGA_ADC_V30_EXPECTED_BUILD_ID;
static unsigned int reads;
static unsigned int status_reads;
static int force_generation_change;
static uint32_t last_read_address;

static void reset_mock(void)
{
  marker = FPGA_ADC_V30_EXPECTED_BUILD_ID;
  reads = 0U;
  status_reads = 0U;
  force_generation_change = 0;
  last_read_address = 0U;
}

uint32_t RegisterRead(uint32_t Address)
{
  reads++;
  last_read_address = Address;
  if (Address == FPGA_ADC_V30_BUILD_ID)
    return marker;
  if (Address == FPGA_ADC_V30_SNAPSHOT_STATUS)
  {
    status_reads++;
    return 0x80000000U | (force_generation_change ? (7U + (status_reads & 1U)) : 7U);
  }
  if (Address == FPGA_ADC_V30_CLOCK_HZ)
    return 122880000U;
  if (Address == FPGA_ADC_V30_EPISODE_STATE)
    return 0x00000301U;
  return 1000U + Address;
}

void RegisterWrite(uint32_t Address, uint32_t Value)
{
  (void)Address;
  (void)Value;
  assert(!"V30 ADC telemetry must be read-only");
}

static char *render_json(const TFPGAADCV30Snapshot *snapshot)
{
  FILE *file = tmpfile();
  long size;
  char *text;

  assert(file != NULL);
  P23PerfTelemetryWriteFPGAADCV30JSON(file, snapshot);
  assert(fflush(file) == 0);
  assert(fseek(file, 0, SEEK_END) == 0);
  size = ftell(file);
  assert(size >= 0);
  assert(fseek(file, 0, SEEK_SET) == 0);
  text = calloc((size_t)size + 1U, 1U);
  assert(text != NULL);
  assert(fread(text, 1U, (size_t)size, file) == (size_t)size);
  fclose(file);
  return text;
}

static void test_pre_v30_does_not_read(void)
{
  TFPGAADCV30Snapshot snapshot;

  reset_mock();
  FPGAADCV30Init(29U);
  assert(!FPGAADCV30Sample());
  FPGAADCV30GetSnapshot(&snapshot);
  assert(reads == 0U);
  assert(!snapshot.Available);
  assert(snapshot.Status == eFPGAADCV30Unsupported);
}

static void test_marker_mismatch_stops_after_marker(void)
{
  TFPGAADCV30Snapshot snapshot;

  reset_mock();
  marker = 0xDEADBEEFU;
  FPGAADCV30Init(30U);
  assert(!FPGAADCV30Sample());
  FPGAADCV30GetSnapshot(&snapshot);
  assert(reads == 1U);
  assert(last_read_address == FPGA_ADC_V30_BUILD_ID);
  assert(!snapshot.Available);
  assert(snapshot.Status == eFPGAADCV30MarkerMismatch);
}

static void test_coherent_snapshot_decode(void)
{
  TFPGAADCV30Snapshot snapshot;
  unsigned int channel;

  reset_mock();
  FPGAADCV30Init(30U);
  assert(FPGAADCV30Sample());
  FPGAADCV30GetSnapshot(&snapshot);
  assert(snapshot.Available);
  assert(snapshot.SnapshotValid);
  assert(snapshot.SnapshotGeneration == 7U);
  assert(snapshot.ClockHz == 122880000U);
  assert(snapshot.EpisodeActive[0]);
  assert(!snapshot.EpisodeActive[1]);
  assert(snapshot.EpisodeValid[0]);
  assert(snapshot.EpisodeValid[1]);
  for (channel = 0; channel < FPGA_ADC_V30_CHANNEL_COUNT; channel++)
  {
    uint32_t offset = 4U * channel;
    assert(snapshot.EpisodeCount[channel] == 1000U + FPGA_ADC_V30_EPISODE_COUNT_BASE + offset);
    assert(snapshot.TotalHighClocks[channel] == 1000U + FPGA_ADC_V30_TOTAL_HIGH_CLOCKS_BASE + offset);
    assert(snapshot.LongestEpisodeClocks[channel] == 1000U + FPGA_ADC_V30_LONGEST_EPISODE_BASE + offset);
    assert(snapshot.LatestEpisodeClocks[channel] == 1000U + FPGA_ADC_V30_LATEST_EPISODE_BASE + offset);
    assert(snapshot.LatestEpisodePeak[channel] == 1000U + FPGA_ADC_V30_LATEST_PEAK_BASE + offset);
  }
}

static void test_generation_change_is_bounded(void)
{
  TFPGAADCV30Snapshot snapshot;

  reset_mock();
  FPGAADCV30Init(30U);
  force_generation_change = 1;
  assert(!FPGAADCV30Sample());
  FPGAADCV30GetSnapshot(&snapshot);
  assert(!snapshot.SnapshotValid);
  assert(snapshot.SnapshotRetryFailureCount == 1U);
  assert(status_reads == 2U * FPGA_ADC_V30_SNAPSHOT_RETRY_LIMIT);
}

static void test_json_contract(void)
{
  TFPGAADCV30Snapshot snapshot;
  char *json;

  reset_mock();
  FPGAADCV30Init(30U);
  assert(FPGAADCV30Sample());
  FPGAADCV30GetSnapshot(&snapshot);
  json = render_json(&snapshot);
  assert(strstr(json, "\"fpga_adc_v30\"") != NULL);
  assert(strstr(json, "\"lifetime_scope\": \"fpga_boot\"") != NULL);
  assert(strstr(json, "\"duration_unit\": \"adc_clocks\"") != NULL);
  assert(strstr(json, "\"episode_count\"") != NULL);
  assert(strstr(json, "\"latest_episode_peak\"") != NULL);
  free(json);
}

int main(void)
{
  test_pre_v30_does_not_read();
  test_marker_mismatch_stops_after_marker();
  test_coherent_snapshot_decode();
  test_generation_change_is_bounded();
  test_json_contract();
  puts("fpga_adc_v30 tests passed");
  return 0;
}
