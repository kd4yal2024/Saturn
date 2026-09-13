#include <assert.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../../common/p23_perf_telemetry.h"

static uint64_t MockElapsedNs(uint64_t StartNs, uint64_t FinishNs)
{
    assert(FinishNs >= StartNs);
    return FinishNs - StartNs;
}

static void AssertAllCountsZero(const TP23SpeakerPacingDiagnostics *Snapshot)
{
    unsigned int Index;

    for(Index = 0; Index < P23_SPEAKER_LOOP_GAP_THRESHOLD_COUNT; Index++)
        assert(Snapshot->LoopGapOverThreshold[Index] == 0U);
    for(Index = 0; Index < P23_SPEAKER_DURATION_THRESHOLD_COUNT; Index++)
    {
        assert(Snapshot->ReceiveDurationOverThreshold[Index] == 0U);
        assert(Snapshot->DMAWriteDurationOverThreshold[Index] == 0U);
    }
}

static void TestNormalOperation(void)
{
    TP23SpeakerPacingDiagnostics Snapshot;

    P23PerfTelemetryInit("p2", 51U);
    P23PerfTelemetrySetSpeakerThread(4321U, SCHED_OTHER, 0, 2);
    P23PerfTelemetryObserveSpeakerLoopGap(MockElapsedNs(1000000U, 2200000U));
    P23PerfTelemetryObserveSpeakerReceiveDuration(MockElapsedNs(3000000U, 3250000U));
    P23PerfTelemetryObserveSpeakerDMAWriteDuration(MockElapsedNs(4000000U, 4300000U));
    P23PerfTelemetryObserveSpeakerQueueDepth(3U);
    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);

    assert(Snapshot.Available);
    assert(Snapshot.ThreadTid == 4321U);
    assert(Snapshot.ThreadSchedulingPolicy == SCHED_OTHER);
    assert(Snapshot.ThreadPriority == 0);
    assert(Snapshot.ThreadCPU == 2);
    assert(Snapshot.MaximumLoopGapNs == 1200000U);
    assert(Snapshot.MaximumReceiveDurationNs == 250000U);
    assert(Snapshot.MaximumDMAWriteDurationNs == 300000U);
    assert(Snapshot.PeakSoftwareQueueFrames == 3U);
    assert(!Snapshot.LastUnderrunValid);
    AssertAllCountsZero(&Snapshot);
}

static void TestInjectedStallsAndIncidentContext(void)
{
    TP23SpeakerPacingDiagnostics Snapshot;
    const uint64_t EventNs = 9876543210ULL;

    P23PerfTelemetryInit("p2", 51U);
    P23PerfTelemetrySetSpeakerThread(7654U, SCHED_OTHER, 0, 1);

    /* Mock a scheduling pause, a socket wakeup stall, and an XDMA stall. */
    P23PerfTelemetryObserveSpeakerLoopGap(MockElapsedNs(1000000U, 18500000U));
    P23PerfTelemetryObserveSpeakerReceiveDuration(MockElapsedNs(20000000U, 29500000U));
    P23PerfTelemetryObserveSpeakerDMAWriteDuration(MockElapsedNs(30000000U, 39000000U));
    P23PerfTelemetryObserveSpeakerQueueDepth(11U);
    P23PerfTelemetrySetSpeakerUnderrunContextWithPacing(
        7U, 0U, 2400U, 3U, false, 17500000U, 9500000U, 9000000U,
        EventNs, 3);
    P23PerfTelemetrySetSpeakerUnderrunRefill(EventNs + 1U, 99U, 99U);
    P23PerfTelemetrySetSpeakerUnderrunRefill(EventNs, 6U, 6U);
    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);

    assert(Snapshot.MaximumLoopGapNs == 17500000U);
    assert(Snapshot.LoopGapOverThreshold[0] == 1U);
    assert(Snapshot.LoopGapOverThreshold[1] == 1U);
    assert(Snapshot.LoopGapOverThreshold[2] == 1U);
    assert(Snapshot.LoopGapOverThreshold[3] == 1U);
    assert(Snapshot.MaximumReceiveDurationNs == 9500000U);
    assert(Snapshot.ReceiveDurationOverThreshold[0] == 1U);
    assert(Snapshot.ReceiveDurationOverThreshold[1] == 1U);
    assert(Snapshot.ReceiveDurationOverThreshold[2] == 1U);
    assert(Snapshot.ReceiveDurationOverThreshold[3] == 1U);
    assert(Snapshot.ReceiveDurationOverThreshold[4] == 0U);
    assert(Snapshot.MaximumDMAWriteDurationNs == 9000000U);
    assert(Snapshot.DMAWriteDurationOverThreshold[0] == 1U);
    assert(Snapshot.DMAWriteDurationOverThreshold[1] == 1U);
    assert(Snapshot.DMAWriteDurationOverThreshold[2] == 1U);
    assert(Snapshot.DMAWriteDurationOverThreshold[3] == 1U);
    assert(Snapshot.DMAWriteDurationOverThreshold[4] == 0U);
    assert(Snapshot.PeakSoftwareQueueFrames == 11U);
    assert(Snapshot.LastUnderrunValid);
    assert(Snapshot.LastUnderrunMonotonicNs == EventNs);
    assert(Snapshot.LastUnderrunLoopGapNs == 17500000U);
    assert(Snapshot.LastUnderrunReceiveDurationNs == 9500000U);
    assert(Snapshot.LastUnderrunDMAWriteDurationNs == 9000000U);
    assert(Snapshot.LastUnderrunFIFOFramesBeforeRefill == 0U);
    assert(Snapshot.LastUnderrunQueuedFrames == 7U);
    assert(Snapshot.LastUnderrunFramesSelected == 6U);
    assert(Snapshot.LastUnderrunFramesWritten == 6U);
    assert(Snapshot.LastUnderrunQueueAgeUs == 2400U);
    assert(Snapshot.LastUnderrunThreadCPU == 3);
}

static void TestInjectedStallDiscrimination(void)
{
    TP23SpeakerPacingDiagnostics Snapshot;

    P23PerfTelemetryInit("p2", 51U);
    P23PerfTelemetryObserveSpeakerLoopGap(MockElapsedNs(1000000U, 18500000U));
    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);
    assert(Snapshot.MaximumLoopGapNs == 17500000U);
    assert(Snapshot.MaximumReceiveDurationNs == 0U);
    assert(Snapshot.MaximumDMAWriteDurationNs == 0U);
    assert(Snapshot.LoopGapOverThreshold[3] == 1U);
    assert(Snapshot.ReceiveDurationOverThreshold[0] == 0U);
    assert(Snapshot.DMAWriteDurationOverThreshold[0] == 0U);

    P23PerfTelemetryInit("p2", 51U);
    P23PerfTelemetryObserveSpeakerReceiveDuration(MockElapsedNs(20000000U, 29500000U));
    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);
    assert(Snapshot.MaximumLoopGapNs == 0U);
    assert(Snapshot.MaximumReceiveDurationNs == 9500000U);
    assert(Snapshot.MaximumDMAWriteDurationNs == 0U);
    assert(Snapshot.LoopGapOverThreshold[0] == 0U);
    assert(Snapshot.ReceiveDurationOverThreshold[3] == 1U);
    assert(Snapshot.DMAWriteDurationOverThreshold[0] == 0U);

    P23PerfTelemetryInit("p2", 51U);
    P23PerfTelemetryObserveSpeakerDMAWriteDuration(MockElapsedNs(30000000U, 39000000U));
    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);
    assert(Snapshot.MaximumLoopGapNs == 0U);
    assert(Snapshot.MaximumReceiveDurationNs == 0U);
    assert(Snapshot.MaximumDMAWriteDurationNs == 9000000U);
    assert(Snapshot.LoopGapOverThreshold[0] == 0U);
    assert(Snapshot.ReceiveDurationOverThreshold[0] == 0U);
    assert(Snapshot.DMAWriteDurationOverThreshold[3] == 1U);
}

static void TestJSONContract(void)
{
    TP23SpeakerPacingDiagnostics Snapshot;
    FILE *File;
    char Buffer[8192];
    size_t Length;

    P23PerfTelemetryGetSpeakerPacingDiagnostics(&Snapshot);
    File = tmpfile();
    assert(File != NULL);
    P23PerfTelemetryWriteSpeakerPacingJSON(File, &Snapshot);
    assert(fseek(File, 0, SEEK_SET) == 0);
    Length = fread(Buffer, 1, sizeof(Buffer) - 1U, File);
    Buffer[Length] = '\0';
    fclose(File);

    assert(strstr(Buffer, "\"speaker_pacing_diagnostics\"") != NULL);
    assert(strstr(Buffer, "\"lifetime_scope\": \"process\"") != NULL);
    assert(strstr(Buffer, "\"duration_unit\": \"microseconds\"") != NULL);
    assert(strstr(Buffer, "\"loop_gap_counts\"") != NULL);
    assert(strstr(Buffer, "\"recvmmsg_duration_counts\"") != NULL);
    assert(strstr(Buffer, "\"dma_write_duration_counts\"") != NULL);
    assert(strstr(Buffer, "\"monotonic_timestamp_ns\": 9876543210") != NULL);
    assert(strstr(Buffer, "\"frames_selected\": 6") != NULL);
    assert(strstr(Buffer, "\"frames_written\": 6") != NULL);
}

int main(void)
{
    TestNormalOperation();
    TestInjectedStallDiscrimination();
    TestInjectedStallsAndIncidentContext();
    TestJSONContract();
    puts("speaker pacing diagnostics tests passed");
    return 0;
}
