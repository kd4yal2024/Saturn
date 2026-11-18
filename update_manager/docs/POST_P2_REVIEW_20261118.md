# Follow-up to the post-P2 performance review

Base: `3423c266` (includes the band-guide feature after the reviewed `a01da27f`).
The deployed G2 was not changed during this source repair.

| Finding | Change | Evidence |
| --- | --- | --- |
| A fast RUN stop/restart can be missed by the DUC/speaker active-flag polls | Controller-lease epochs are read with the address match. Each ingress resets its sequence and software queue on an epoch change before accepting data. | Actual DUC and speaker ingress fixtures fail against the old source and pass against the repair, including ASan/UBSan. See P2_V44_SEQUENCE_LIFECYCLE.md. |
| Old rejected band status is reported as a source-selection rejection | Record the previous settings result at the source request; only a changed terminal rejection with no pending settings is reported for that request. | Callback regression ignores the old rejection and reports a new one. |
| SWL-panel 27 MHz/FM operating memories have no edge markers | One panel-edge selector includes those two existing operating ranges alongside SWL broadcast ranges. | Unit tests and real Chrome band-button checks cover both spectrum and waterfall. |
| Concurrent final sender drops can miss the owner wake | An atomic sender count selects exactly one final drop, publishes disconnect and then notifies. | Concurrent-close, parked-owner, FIFO, contention and retained-envelope tests. |
| Output-permit acquisition copied into four P2 send paths | Shared `enter_output` helper; caller still owns the returned guard across the same send. | Default/managed Bridge suites, including P2 stop, sequencing and output fences. |
| Template duplicates the operating band-edge table | Export the existing TypeScript table and use it after bundle API initialization. | Template seam, web suite and actual Chrome initialization/band tests. |

The supplied 60-second G2 performance measurements remain the evidence for the
previous poller changes; this follow-up does not claim new performance gains.
No polling intervals, priorities, FIFO thresholds, RF output limits or PS
processing were changed.

The rejected SATP sender still needs network/source identification; an aggregate
rejection counter cannot identify its reason. Hosted CI disablement is also an
external issue. Neither is represented as fixed by this patch.
