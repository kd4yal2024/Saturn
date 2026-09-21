export interface SatpState {
  source: 'tci' | 'satp'; enabled: boolean; paired: boolean; ready: boolean;
  health: string; peakDb: number; bufferFrames: number; missingPackets: number;
  generation: number; pending: boolean; receivedAt: number;
}

export function parseSatpState(args: readonly string[], now = performance.now()): SatpState | null {
  if (args.length !== 10 || !['tci', 'satp'].includes(args[0] ?? '')) return null;
  if (![1, 2, 3, 9].every(i => ['true', 'false'].includes(args[i] ?? ''))) return null;
  if (![5, 6, 7, 8].every(i => args[i] != null && args[i] !== '' && Number.isFinite(Number(args[i])))) return null;
  if (!['unpaired', 'waiting', 'lost', 'degraded', 'healthy'].includes(args[4] ?? '')) return null;
  return { source: args[0] as SatpState['source'], enabled: args[1] === 'true', paired: args[2] === 'true',
    ready: args[3] === 'true', health: args[4]!, peakDb: Number(args[5]), bufferFrames: Number(args[6]),
    missingPackets: Number(args[7]), generation: Number(args[8]), pending: args[9] === 'true', receivedAt: now };
}

export function satpTxBlockReason(s: SatpState | undefined, now = performance.now()): string | null {
  if (!s) return null; // legacy bridges keep their browser microphone path
  if (now - s.receivedAt > 1500) return 'TX audio source status is stale';
  if (s.pending) return 'Waiting for TX source acknowledgement';
  if (s.source !== 'satp') return null;
  if (!s.enabled || !s.paired) return 'Pair the native SATP sender before TX';
  if (!s.ready) return 'Native SATP audio is not arriving';
  return null;
}

export function txUsesBrowserMic(s: SatpState | undefined, useMic = true): boolean {
  return useMic && s?.source !== 'satp';
}
