#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../../common/fpga_fifo_v29.h"
#include "../../common/p23_perf_telemetry.h"

static uint32_t marker = FPGA_FIFO_V29_EXPECTED_BUILD_ID;
static uint16_t generation = 9U;
static unsigned int reads;
static unsigned int writes;
static unsigned int control_reads;
static uint32_t last_read_address;
static uint32_t last_write_address;
static uint32_t last_write_value;
static int force_timeout;

static void reset_mock(void)
{
  marker = FPGA_FIFO_V29_EXPECTED_BUILD_ID;
  generation = 9U;
  reads = 0U;
  writes = 0U;
  control_reads = 0U;
  last_read_address = 0U;
  last_write_address = 0U;
  last_write_value = 0U;
  force_timeout = 0;
}

uint32_t RegisterRead(uint32_t Address)
{
  reads++;
  last_read_address = Address;
  if (Address == FPGA_FIFO_V29_BUILD_ID)
    return marker;
  if (Address == FPGA_FIFO_V29_SNAPSHOT_CONTROL)
  {
    control_reads++;
    if (force_timeout || control_reads <= 2U)
      return 0x80000000U | generation;
    return 0x80000000U | (uint16_t)(generation + 1U);
  }
  return 1000U + Address;
}

void RegisterWrite(uint32_t Address, uint32_t Value)
{
  writes++;
  last_write_address = Address;
  last_write_value = Value;
}

static char *render_json(const TFPGAFifoV29Snapshot *snapshot)
{
  FILE *file = tmpfile();
  long size;
  char *text;

  assert(file != NULL);
  P23PerfTelemetryWriteFPGAFifoV29JSON(file, snapshot);
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

static void test_pre_v29_does_not_read(void)
{
  TFPGAFifoV29Snapshot snapshot;
  reset_mock();
  FPGAFifoV29Init(28U);
  assert(!FPGAFifoV29Sample());
  FPGAFifoV29GetSnapshot(&snapshot);
  assert(reads == 0U);
  assert(writes == 0U);
  assert(!snapshot.Available);
  assert(snapshot.Status == eFPGAFifoV29Unsupported);
}

static void test_marker_mismatch_stops_after_marker(void)
{
  TFPGAFifoV29Snapshot snapshot;
  reset_mock();
  marker = 0xDEADBEEFU;
  FPGAFifoV29Init(29U);
  assert(!FPGAFifoV29Sample());
  FPGAFifoV29GetSnapshot(&snapshot);
  assert(reads == 1U);
  assert(last_read_address == FPGA_FIFO_V29_BUILD_ID);
  assert(writes == 0U);
  assert(!snapshot.Available);
  assert(snapshot.Status == eFPGAFifoV29MarkerMismatch);
  assert(snapshot.BuildId == marker);
}

static void test_snapshot_decoding_and_generation(void)
{
  TFPGAFifoV29Snapshot snapshot;
  unsigned int channel;
  reset_mock();
  FPGAFifoV29Init(29U);
  assert(FPGAFifoV29Sample());
  FPGAFifoV29GetSnapshot(&snapshot);
  assert(snapshot.Available);
  assert(snapshot.SnapshotValid);
  assert(snapshot.SnapshotGeneration == 10U);
  assert(writes == 1U);
  assert(last_write_address == FPGA_FIFO_V29_SNAPSHOT_CONTROL);
  assert(last_write_value == 1U);
  for (channel = 0U; channel < FPGA_FIFO_V29_CHANNEL_COUNT; channel++)
  {
    assert(snapshot.OccupancyWords[channel] == 1000U + FPGA_FIFO_V29_OCCUPANCY_BASE + 4U * channel);
    assert(snapshot.MinimumWords[channel] == 1000U + FPGA_FIFO_V29_MINIMUM_BASE + 4U * channel);
    assert(snapshot.MaximumWords[channel] == 1000U + FPGA_FIFO_V29_MAXIMUM_BASE + 4U * channel);
    assert(snapshot.EventTransitions[channel] == 1000U + FPGA_FIFO_V29_EVENTS_BASE + 4U * channel);
  }
}

static void test_timeout_is_bounded(void)
{
  TFPGAFifoV29Snapshot snapshot;
  reset_mock();
  force_timeout = 1;
  FPGAFifoV29Init(29U);
  assert(!FPGAFifoV29Sample());
  FPGAFifoV29GetSnapshot(&snapshot);
  assert(!snapshot.SnapshotValid);
  assert(snapshot.SnapshotTimeoutCount == 1U);
  assert(control_reads == FPGA_FIFO_V29_SNAPSHOT_POLL_LIMIT + 1U);
  assert(writes == 1U);
  assert(last_write_value == 1U);
}

static void test_json_names_and_word_units(void)
{
  TFPGAFifoV29Snapshot snapshot;
  char *json;
  reset_mock();
  FPGAFifoV29Init(29U);
  assert(FPGAFifoV29Sample());
  FPGAFifoV29GetSnapshot(&snapshot);
  json = render_json(&snapshot);
  assert(strstr(json, "\"fpga_fifo_v29\"") != NULL);
  assert(strstr(json, "\"occupancy_words\"") != NULL);
  assert(strstr(json, "\"minimum_words\"") != NULL);
  assert(strstr(json, "\"maximum_words\"") != NULL);
  assert(strstr(json, "\"event_transitions\"") != NULL);
  assert(strstr(json, "sample_loss") == NULL);
  free(json);
}

int main(void)
{
  test_pre_v29_does_not_read();
  test_marker_mismatch_stops_after_marker();
  test_snapshot_decoding_and_generation();
  test_timeout_is_bounded();
  test_json_names_and_word_units();
  puts("fpga_fifo_v29 tests passed");
  return 0;
}
