import { describe, expect, it } from "vitest";
import { TelemetryFreshness } from "../src/puresignal/freshness";

const stamp = (observed = "1020000000", sequence = "1") => ({
  bootId: "boot-a", observedMonoNs: observed, publishedMonoNs: observed,
  sequence, staleAfterMs: 200,
});
const ready = () => {
  const clock = new TelemetryFreshness();
  clock.reconnect("1");
  clock.probe("nonce-a", "0");
  expect(clock.acceptProbe({ nonce: "nonce-a", connectionEpoch: "1",
    bootId: "boot-a", serverReplyMonoNs: "1010000000" }, "20000000")).toBe(true);
  return clock;
};

describe("PureSignal telemetry freshness (delivery included)", () => {
  it("is unavailable until a matching nonce round trip, not fresh on receipt", () => {
    const clock = new TelemetryFreshness();
    clock.reconnect("1");
    expect(clock.check("monitor", "1", stamp(), "0").validity).toBe("unavailable");
    clock.probe("nonce-a", "0");
    expect(clock.acceptProbe({ nonce: "wrong", connectionEpoch: "1", bootId: "boot-a",
      serverReplyMonoNs: "1010000000" }, "20000000")).toBe(false);
  });
  it("bounds transit plus publication delay without assuming symmetric RTT", () => {
    const result = ready().check("monitor", "1", stamp(), "40000000");
    expect(result.validity).toBe("valid");
    expect(result.maximumAgeMs).toBeGreaterThanOrEqual(30);
  });
  it("does not let a just-received queued packet look fresh", () => {
    const clock = ready();
    clock.check("monitor", "1", stamp(), "100000000");
    const result = clock.check("monitor", "1", stamp("1020000000", "2"), "450000000");
    expect(result.validity).toBe("stale");
    expect(result.maximumAgeMs).toBeGreaterThan(400);
  });
  it("retires the bound on a stalled consumer and discards queued pre-barrier data", () => {
    const clock = ready();
    expect(clock.check("monitor", "1", stamp(), "5000000000").reason).toBe("timing-unverified");
    clock.probe("nonce-b", "5010000000");
    expect(clock.acceptProbe({ nonce: "nonce-b", connectionEpoch: "1", bootId: "boot-a",
      serverReplyMonoNs: "6020000000" }, "5030000000")).toBe(true);
    expect(clock.check("monitor", "1", stamp(), "5030000000").reason)
      .toBe("before-freshness-barrier");
    expect(clock.check("monitor", "1", stamp("6040000000", "2"), "5050000000").validity)
      .toBe("valid");
  });
  it("rejects a queued probe reply and connection/boot/sequence reuse", () => {
    const clock = ready();
    expect(clock.check("monitor", "1", stamp(), "40000000").validity).toBe("valid");
    expect(clock.check("monitor", "1", stamp(), "50000000").reason).toBe("reordered");
    clock.reconnect("2");
    expect(clock.check("monitor", "1", stamp(), "50000000").reason).toBe("connection-mismatch");
    clock.probe("nonce-c", "50000000");
    expect(clock.acceptProbe({ nonce: "nonce-c", connectionEpoch: "2", bootId: "boot-a",
      serverReplyMonoNs: "1100000000" }, "1000000000")).toBe(false);
    const other = ready();
    expect(other.check("monitor", "1", { ...stamp(), bootId: "boot-b" }, "40000000").reason)
      .toBe("generation-mismatch");
  });
  it("expires even if the consumer remains active and rejects malformed stamps", () => {
    const clock = ready();
    for (let i = 1; i <= 5; i++) clock.check("monitor", "1", stamp(), `${i * 400000000}`);
    expect(clock.check("monitor", "1", stamp(), "2200000000").reason).toBe("timing-unverified");
    expect(ready().check("monitor", "1", stamp("NaN"), "40000000").reason).toBe("invalid-stamp");
    expect(ready().check("monitor", "1", stamp("18446744073709551616"), "40000000").reason)
      .toBe("invalid-stamp");
    expect(ready().check("monitor", "1", { ...stamp(), publishedMonoNs: "99999999999" }, "40000000").reason)
      .toBe("invalid-stamp");
  });
});
