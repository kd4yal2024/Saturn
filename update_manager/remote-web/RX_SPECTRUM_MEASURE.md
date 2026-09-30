# RX spectrum measurement in Saturn Remote

The **RX Measure** button in the Panadapter / Waterfall toolbar opens a local browser measurement panel. It observes the raw RX IQ FFT before display averaging, trace smoothing, peak hold, waterfall cleanup, auto range, color mapping, or zoom. It does not send a radio command or change RX settings. The measurement is available while the display uses raw IQ in a narrowband RX mode; the server spectrum display path and WFM are not accepted.

## Capture and compare

1. Receive on a stable frequency with a fixed antenna, ADC, attenuation, RF gain, mode, and filter. Keep the IQ sample rate the same in each run. The 22/Q24 observation used 80 m, attenuation 10 dB, and RF/AGC gain 63.
2. Open **RX Measure**, enter the current firmware label, select the same duration for both runs (30 or 120 seconds), and select **Start capture**. The label is operator supplied; the browser does not verify the FPGA USR_ACCESS ID.
3. Download the JSON after the capture finishes. Repeat under the other firmware with the same RX settings. A setting, stream, or display source change during capture aborts it. A sparse or interrupted capture is marked `limited`.
4. Load both JSON files in **Baseline JSON** and **Candidate JSON**, then select **Compare captures**. The panel refuses mismatched RX settings, FFT size, or duration. Reported differences are candidate minus baseline.

The JSON contains the receive settings, timestamps, sample count, coverage quality, relative full-span mean spectrum, and three summary measurements:

- **Adjacent noise floor:** median of each frame's 20th-percentile FFT bin level within 10 kHz of center, excluding the RX passband, a 250 Hz passband guard, and the center 250 Hz. This is an adjacent-band estimate, not a calibrated receiver noise figure.
- **Passband over floor:** median passband mean-power level minus the adjacent floor. It includes noise and signals; it is not a decoded-speech intelligibility score.
- **Wideband burst rise:** 99th-percentile minus median full-span power across sampled FFT frames. It helps quantify static crashes captured during the run. The 10-frame/s sampler can miss brief impulses.

All levels are **relative FFT dB**, not dBm. The FFT's Hann window and bin width affect the numeric floor, so only compare captures with matching sample rate and FFT size. Waterfall palette and range cannot influence these numbers. Browser audio underruns also cannot change the RX IQ spectrum, but they can affect what the operator hears.

On-air A/B results can be dominated by propagation, station activity, and atmospheric noise. The strongest firmware comparison uses a repeatable RF source and a controlled off-channel blocker, measuring the change in adjacent floor and unwanted products under the same input levels. Existing 18/Q20 soak counters do not contain the spectrum data needed for this comparison; new captures are required under both images.
