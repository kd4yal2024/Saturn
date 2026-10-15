/** M2 offline contract, NOT a production-enabled TX feature.
 * Finite-number, length, enum, authority and generation checks are runtime
 * requirements; these types alone do not enforce them. Never JSON-encode NaN.
 */
export type U64 = string; // canonical unsigned decimal; no JS precision loss
export type Backend = "p2" | "xdma";
export type Owner = "bridge" | "external-p2-client" | "none";
export type Panel = "ps-monitor" | "distortion-analyzer" | "amp-view";
export type Reason =
  | "unsupported-backend" | "native-abi-unverified" | "no-controller"
  | "external-controller" | "not-keyed" | "settling" | "no-feedback"
  | "stale" | "generation-mismatch" | "config-changed" | "unequal-length"
  | "non-finite" | "wrong-format" | "wrong-rate" | "wrong-route"
  | "sequence-gap" | "duplicate" | "reordered" | "dma-framing"
  | "fifo-loss" | "queue-overrun" | "alignment-unverified"
  | "adc-overrange" | "ddc-clamp" | "clip-status-unknown"
  | "severe-overdrive" | "solution-rejected" | "no-calibration"
  | "snapshot-race" | "out-of-band" | "unresolved-noise-floor"
  | "tone-missing" | "tone-unbalanced" | "native-count-out-of-range"
  | "timing-unverified" | "connection-mismatch" | "observation-incomplete"
  | "counter-reset" | "counter-inexact";

export interface Stamp {
  bootId: string; generation: U64; configId: string; sequence: U64;
  observedMonoNs: U64; publishedMonoNs: U64;
  observedUtc: string | null; // ISO8601; UTC may be unsynchronized
  utcSynchronized: boolean;
  capture: {
    firstSampleOrdinal: U64 | null; pairs: number; rateHz: number;
    fpgaTicks: U64 | null; fpgaClockHz: number | null;
    basis: "hardware-ticks" | "stream-relative" | "unavailable";
    // Ordinals are host-derived within a generation, NOT FPGA timestamps.
  } | null;
}
export type Product<T> = {
  stamp: Stamp;
  reasons: Reason[]; ageAtPublishMs: number | null; staleAfterMs: number;
  // Last-good values may be retained separately and visibly marked historical.
} & ({ validity: "valid"; data: T } |
     { validity: "stale" | "invalid" | "unavailable"; data: null });
/** A nonce probe bounds serverMono-clientMono by [serverAtReply-clientReceive,
 * serverAtReply-clientSend]. Use the UPPER bound to compute maximum current
 * age from stamp.observedMonoNs, including ALL transit/queue delay. Never use
 * ageAtPublishMs + timeSinceReceipt as a freshness certificate. Bounds expire;
 * reconnect/stall retires the connection epoch and requires a fresh probe.
 * Rates below assume a conservative oscillator-error budget in both clocks. */
export interface ClockProbe {
  nonce: string; connectionEpoch: U64; bootId: string;
  clientSendMonoNs: U64; clientReceiveMonoNs: U64; serverReplyMonoNs: U64;
}
export interface DeliveryEnvelope<T> {
  connectionEpoch: U64; product: Product<T>;
}
/** [start,end) in the SAME source clock/ordinal basis as the measured capture.
 * A valid clear counter bracket must contain the ENTIRE capture, remain in one
 * generation/config, and include a verified non-resetting monotonic counter.
 * Unarmed/unavailable counters and instantaneous clear bits are not coverage.
 * Nonzero deltas over a wider bracket conservatively invalidate the capture;
 * they do not locate the overload inside the capture. */
export interface ObservationInterval {
  sourceBootId: string; sourceGeneration: U64; configId: string;
  firstSampleOrdinal: U64; endSampleOrdinalExclusive: U64;
  basis: "stream-relative" | "hardware-ticks";
}
export interface OverloadEvidence {
  interval: ObservationInterval | null;
  observedMonoNs: U64;
  validity: "valid" | "invalid" | "unavailable"; reasons: Reason[];
  coverage: "complete-counter-bracket" | "instantaneous-only" | "unavailable";
  source: "adc1-overrange" | "adc2-overrange" | "ddc0-clamp" | "ddc1-clamp";
  counterStart: U64 | null; counterEnd: U64 | null;
  counterGeneration: U64 | null;
  endCounterGeneration: U64 | null;
  counterWidthBits: number | null; // source-qualified integer 1..64
  counterSaturated: boolean | null; // covers interval; null => cannot certify clear
  counterOverflow: boolean | null; // covers interval; null => cannot certify clear
  detected: boolean | null;
}
export interface Identity {
  bridgeCommit: string; firmware: string; fpgaBuildId: string;
  firmwareArtifactSha256: string | null;
  wdspCommit: string | null; wdspVersion: number | null;
  nativeArchiveSha256: string | null; portCommit: string | null;
  patchedSourceManifestSha256: string | null; bridgeBinarySha256: string | null;
  nativeVerified: boolean;
}
export interface Route {
  profileId: string; profileRevision: U64;
  kind: "internal-pa" | "external-amplifier";
  txAntenna: "ANT1" | "ANT2" | "ANT3";
  feedbackPhysicalAdc: "ADC1" | "ADC2"; // NOT protocol adc=2 TX-reference source
  feedbackConnector: string | null; externalCouplerId: string | null;
  feedbackAttenuationDb: number; attestation: "configured" | "verified";
  txFrequencyHz: number; driveSettingWatts: number; bandId: string;
  // driveSettingWatts is a setting, not calibrated measured RF power.
}

/** Internal descriptor backed by a bounded reusable buffer pool.
 * Typed arrays are internal, not the browser wire format. */
export interface FeedbackPairBlock {
  identity: Identity; route: Route; stamp: Stamp; backend: Backend;
  txEpoch: U64; format: "iq-f64-s24-div-8388608";
  rateHz: 192000; complexPairs: number;
  txReferenceIQ: Float64Array; rfFeedbackIQ: Float64Array; // each = 2*pairs
  order: "I,Q";
  hardwarePairing: "interleaved-ddc-even-odd-shared-lo";
  alignment: "configured" | "replay-verified" | "qualified";
  transportSequence: number | null; // P2 u32; XDMA header has no sequence
  continuity: "continuous" | "new-generation" | "discontinuity";
  gapPairs: U64 | null; // null when exact source loss cannot be located
  interval: ObservationInterval;
  overloadEvidence: OverloadEvidence[];
  requestedDelaySeconds: number; actualDelaySeconds: number | null;
  hardwarePeak: number; // normalized reference magnitude, never watts
}
export interface PairRejection { stamp: Stamp; txEpoch: U64; reasons: Reason[]; }
export interface PsConfig {
  expectedGeneration: U64; expectedConfigId: string;
  enabled: boolean; calibration: "automatic" | "single";
  autoAttenuate: boolean; attenuationDb: number;
  hardwarePeak: number; requestedDelaySeconds: number;
  moxSettlingSeconds: number; calibrationLoopDelaySeconds: number;
  route: Route;
}
export type PsCommand =
  | { op: "configure"; requestId: string; config: PsConfig }
  | { op: "reset"; requestId: string; expectedGeneration: U64 }
  | { op: "calibrate-once"; requestId: string; expectedGeneration: U64 };
// There is deliberately no key/PTT operation in PsCommand or plot subscriptions.
export interface CommandResult {
  requestId: string; accepted: boolean; generation: U64; configId: string;
  reasons: Reason[]; applied: PsConfig | null;
  actualDelaySeconds: number | null;
}
export interface ControllerStatus {
  owner: Owner; backend: Backend; enabled: boolean;
  state: "off" | "waiting" | "settling" | "collecting" | "calculating"
    | "correcting" | "bypassed" | "fault" | "unsupported";
  keyedByTxAuthority: boolean; correctionAppliedReported: boolean | null;
  calibrationCompatible: boolean; faultReasons: Reason[];
  identity: Identity; route: Route;
  infoRaw: [number, number, number, number, number, number, number, number,
            number, number, number, number, number, number, number, number];
  infoDecodeId: "wdsp-b02d5bac-ps3";
  feedbackLevelRaw: number; // info[4], dimensionless native code
  completedCalibrations: number; attemptedCalibrations: number;
  maxReferenceMagnitude: number; hardwarePeak: number;
  requestedDelaySeconds: number; actualDelaySeconds: number | null;
  moxSettlingSeconds: number; calibrationLoopDelaySeconds: number;
  acceptedPairs: U64; rejectedBlocks: U64; transportGaps: U64;
  queueDepthBlocks: number; queueCapacityBlocks: number;
  queueDelayMs: number; lastPairAgeMs: number | null;
  overloadEvidence: OverloadEvidence[];
}
export interface AmpView {
  representation: "wdsp-inverse-normalized-with-explicit-transform";
  nativeMeasured: { x: number[]; ym: number[]; yc: number[]; ys: number[] };
  nativeCorrection: { xm: number[]; ym: number[]; xa: number[]; yaDeg: number[] };
  phaseReferenceDeg: number; measuredCount: number; correctionCount: number;
  measuredGainDb: (number | null)[]; measuredPhaseDeg: (number | null)[];
  measuredInputNormalized: number[]; measuredOutputNormalized: number[];
  correctionGainDb: (number | null)[];
  solutionEpoch: U64 | null; calibrationCapture: Stamp["capture"];
  snapshotConsistency: "verified" | "bracketed-only";
  // Native API supplies no timestamp or generation; do not invent either.
}
export interface BandPower { lowHz: number; highHz: number; linearPower: number; }
export interface ImdSide {
  frequencyHz: number; integration: BandPower | null;
  dbcWeakerTone: number | null;
  availability: "resolved" | "upper-bound" | "unavailable";
  upperBoundDbc: number | null; reasons: Reason[];
}
export interface SpectrumCapture {
  source: "raw-rf-feedback-pre-rx-dsp-pre-pscc-delay";
  interval: ObservationInterval; overloadEvidence: OverloadEvidence[];
  rateHz: number; centerHz: number; fftSize: number;
  window: string; coherentGain: number; enbwBins: number;
  averaging: "linear-power"; averagedFfts: number; durationMs: number;
  dbScale: "relative-dBFS-bin-power"; spectrumDb: number[];
  usableLowHz: number; usableHighHz: number;
  noiseFloorLinearPower: number; noiseEstimator: string;
  route: Route;
  absolutePowerCalibrationId: string | null; // absent => do not display dBm/W
}
export interface TwoToneMeasurement {
  mode: "two-tone-imd";
  reference: "negative-dBc-relative-to-weaker-fundamental";
  tone1: BandPower; tone2: BandPower; toneBalanceDb: number;
  tone1Hz: number; tone2Hz: number;
  products: { order: 3 | 5 | 7; lower: ImdSide; upper: ImdSide;
              worstResolvedDbc: number | null }[];
  dbcPep: { order: 3 | 5 | 7; lower: number | null; upper: number | null }[];
}
export interface BroadbandMeasurement {
  mode: "broadband-regrowth";
  reference: "integrated-in-band-linear-power";
  excitation: "speech" | "noise" | "other";
  inBand: BandPower;
  regrowthBands: { power: BandPower; relativeToInBandDb: number | null }[];
  adjacentBands: { power: BandPower; relativeToInBandDb: number | null }[];
  // Deliberately no fundamentals, tone balance, IMD orders, or PEP inference.
}
export interface SpectrumMeasurement {
  capture: Product<SpectrumCapture>;
  twoTone: Product<TwoToneMeasurement>;
  broadband: Product<BroadbandMeasurement>;
  // Independent validity: broadband may be valid while twoTone is unavailable.
}
export interface PanelTelemetry {
  schema: "saturn-puresignal-offline-v2";
  monitor: Product<ControllerStatus>;
  distortionAnalyzer: Product<SpectrumMeasurement>;
  ampView: Product<AmpView>;
}
export interface PlotSubscription {
  observerSessionId: string; panels: Panel[]; plotsHz: 0 | 1 | 2;
  // authenticated observer; no controller lease, settings, or TX authority
}
export interface ControllerPort {
  apply(command: PsCommand, authenticatedControllerId: string): CommandResult;
  ingest(block: FeedbackPairBlock): "accepted" | PairRejection;
  onTxEpochChanged(epoch: U64, keyed: boolean): void; // existing authority only
  invalidate(reason: Reason): void;
  snapshot(): PanelTelemetry;
}
