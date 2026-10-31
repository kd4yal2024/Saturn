/** Digital WDSP output only. Receipt freshness is not RF/sample-time proof. */
export const TX_DISPLAY_STALE_MS = 750;

export function acceptsTxDisplayFrame(connected: boolean, active: boolean,
  rate: number, floats: number, bytes: number): boolean {
  return connected && active && rate === 192000 && Number.isInteger(floats)
    && floats >= 64 && floats <= 8192 && floats % 2 === 0 && bytes >= 64 + floats * 4;
}

export function txDisplayStatus(connected: boolean, active: boolean, paused: boolean,
  lastAt: number | null, now: number): string {
  if (!connected) return 'TX display unavailable — disconnected';
  if (!active) return 'RX — TX baseband display starts with transmission';
  if (paused) return 'TX display paused — trace is not live';
  if (lastAt === null) return 'Waiting for TX baseband IQ — no RF feedback displayed';
  const age = now - lastAt;
  if (age < 0 || age > TX_DISPLAY_STALE_MS) return 'TX display stale — no fresh TX IQ; not RF feedback';
  return 'Live TX baseband (WDSP) — delivery age unverified; not RF feedback or calibrated RF IMD';
}

/** TX frames are contiguous snapshots separated by omitted time. Never splice
 * those gaps into an FFT as if they were continuous samples. */
export function latestTxDisplayWindow(packets: readonly Float32Array[]): Float32Array | null {
  return packets.length ? packets[packets.length - 1]! : null;
}
