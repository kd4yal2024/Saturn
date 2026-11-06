/** Retained packets are immutable, main-owned copies. A plan pins only the
 * selected packet references until submitted/replaced; it never detaches them.
 * Times describe browser ingress, not the FPGA observation interval. */
export function planDisplayIqWindow(packets: readonly Float32Array[], desiredPairs: number,
  tx: boolean, arrival: (packet: Float32Array) => number) {
  if (!packets.length) return null;
  const selected: { packet: Float32Array; start: number; take: number; arrivalMs: number }[] = [];
  const limit = tx ? packets.at(-1)!.length : Math.max(0, Math.floor(desiredPairs)) * 2;
  let floatCount = 0;
  for (let i = packets.length - 1; i >= 0 && floatCount < limit; i--) {
    const packet = packets[i]!;
    const take = Math.min(packet.length - packet.length % 2, limit - floatCount);
    if (!take) continue;
    selected.push({packet, start:packet.length-take, take, arrivalMs:arrival(packet)});
    floatCount += take;
    if (tx) break;
  }
  if (!floatCount) return null;
  selected.reverse();
  return {floatCount, oldestIngressMs:Math.min(...selected.map(p=>p.arrivalMs)),
    newestIngressMs:Math.max(...selected.map(p=>p.arrivalMs)),
    write(destination: Float32Array) {
      if (destination.length < floatCount) throw new RangeError('Display window destination too small');
      let offset = 0;
      for (const part of selected) {
        destination.set(part.packet.subarray(part.start,part.start+part.take),offset);
        offset += part.take;
      }
      return offset;
    }};
}
