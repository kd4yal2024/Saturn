# TX audio and requested power repair, 8 November 2026

The installed `8b5d8402` journal records voice keying at 40 commanded watts
(DAC 41) and then 100 commanded watts (DAC 68), followed by `QueueOverrun`
at **audio-ingress**. Cleanup completed. That evidence identifies the stop
boundary; it does not establish an RF-power or SWR trip.

## Changes

- Keep an unfinished power-slider edit separate from committed host state.
  Host status updates cannot replace a requested 50 watts with the previous
  40 watts before the slider's change event sends it. Band/profile recall and
  disconnect discard the unfinished edit. Diagnostics include committed watts.
- Ordinary live browser PCM retains at most four queued blocks. A producer
  burst replaces the oldest block only within the same output generation,
  revision and source. Original arrival times remain intact. A counted skip
  clears older local microphone staging without refreshing watchdog anchors.
  The UI reports cumulative PCM skips. This can cause a brief audio gap;
  it avoids accumulating old speech or stopping RF solely for a fresh burst.
- SATP, diagnostic stimuli, experimental Opus, configuration and feedback
  queues retain their existing contracts. Retired callbacks cannot evict new
  speech or fault a new generation. Stale sources, release, disconnect and
  authority checks still stop output; no deadline is increased.

## Power calibration

The saved G2 PA profile was `legacy`. Near 27 MHz, its 40/50/100-watt commands
produce DAC codes 41/46/68. The existing saved-Thetis G2 starting profile
produces 68/76/108. A power target is not a measured watt reading. Applying
that existing profile must be explicit, backed up and verified on the LP
meter; it is a starting profile, not a new calibration of this individual PA.
Do not compensate by raising generator amplitude. Preserve the normal
1–100-watt control and all other preferences when applying the profile.

For balanced two-tone signals average power is half PEP. Establish the LP
meter mode before comparing its reading with the requested target. Compare
the radio's output separately from external-amplifier output.

## Verification

The actual shared ingress and P2/Direct XDMA output adapters are exercised
with a 100-watt software target, using loopback sockets and ordinary files.
A 48-block burst, release during pressure, retired-generation callback and
an expired source cover continued output, complete cleanup and fresh re-arm.
The real-WDSP replay covers both adapters. These are offline tests, not RF
or PureSignal qualification.

An aggregate-suite failure exposed a separate fixture clock error: the
combined native handoff replay advanced synthetic milliseconds in a tight
loop while Direct prefill used elapsed wall time. It could mark its own
queued IQ stale, panic and deliberately retain native channel 1, blocking
later tests. The harness now sleeps one millisecond per simulated millisecond
while awaiting key, matching its existing cleanup pacing. Production timing
and native shutdown are unchanged.

Firmware, driver, RXC1 polling and PureSignal curve-retention work are outside
this repair. Real watt output and speech behavior still require the attended
LP-meter check after deployment.
