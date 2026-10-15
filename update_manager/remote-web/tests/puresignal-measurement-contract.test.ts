import { expect, it } from "vitest";
import type { BroadbandMeasurement, Product, TwoToneMeasurement } from "../../docs/puresignal/contracts";

const speech: BroadbandMeasurement = {
  mode: "broadband-regrowth", reference: "integrated-in-band-linear-power", excitation: "speech",
  inBand: { lowHz: 0, highHz: 4000, linearPower: 0.2 },
  regrowthBands: [], adjacentBands: [],
};
// Compiler negative checks are part of `npm run typecheck`, not runtime claims.
// @ts-expect-error speech is NOT a two-tone IMD measurement
const notTwoTone: TwoToneMeasurement = speech;
const unavailable: Product<TwoToneMeasurement> = {
  stamp: { bootId: "a", generation: "1", configId: "speech", sequence: "1",
    observedMonoNs: "1", publishedMonoNs: "2", observedUtc: null, utcSynchronized: false, capture: null },
  validity: "unavailable", reasons: ["tone-missing"], ageAtPublishMs: 0, staleAfterMs: 1500, data: null,
};
// @ts-expect-error nonvalid products must not carry a last-good value as current
const invalidButLooksCurrent: Product<BroadbandMeasurement> = { ...unavailable, data: speech };

it("represents speech regrowth without invented fundamentals or tone validity", () => {
  expect(speech.mode).toBe("broadband-regrowth");
  expect("tone1" in speech).toBe(false);
  expect("tone2" in speech).toBe(false);
  expect("products" in speech).toBe(false);
  expect(unavailable.data).toBeNull();
  // Explicitly keep negative compile fixtures live without treating them as API output.
  expect(notTwoTone).toBe(speech);
  expect(invalidButLooksCurrent.validity).toBe("unavailable");
});
