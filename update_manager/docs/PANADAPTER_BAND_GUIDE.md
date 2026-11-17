# Panadapter band guidance and SWL schedules

Display Settings provides three options, enabled by default:

- **Ham / SWL Band Segments**: colored US ham operating segments and separate
  SWL broadcast ranges, on both the spectrum and waterfall. Blue denotes CW,
  gold data, green phone, purple beacons, orange FM and cyan broadcasting.
  Text and frequency tooltips accompany the colors.
- **SWL Station Labels**: station names and frequency ticks on the receive
  panadapter. Hover a name for the grouped station listings, country/language,
  UTC schedule and data edition. Names sharing a carrier appear as one label
  with a count; their full listings are in the tooltip.
- **Scheduled Now (UTC)**: filter by the database's UTC times, supported day
  codes and applicable start/stop dates. Turning it off shows other listed
  schedules too. Irregular/special day codes are not asserted to be current.

These choices use the existing display-preferences path: band memories,
profiles, host settings and the browser fallback retain them. Partial appearance
updates preserve the choices. The appearance preset does not reset them.

The overlay runs independently of TX controls, permission checks, audio, DSP,
capture data and band-memory frequency classification. The 27 MHz operating
memory is labeled separately from amateur allocations and SWL 11m broadcasting.

## Data and provenance

Ham guidance uses the [ARRL US band plan](https://www.arrl.org/band-plan), reviewed
2026-11-17, with the US mode-boundary context in the
[ARRL band chart](https://www.arrl.org/files/file/Regulatory/Band%20Chart/Hambands4_Grayscale_11x8_5.pdf).
It describes operating segments, not an individual license class's privileges.
The current 60m guide draws four narrow channels and the 15 kHz segment rather
than coloring the entire navigation range as one allocation.

The bundled station data is **EiBi B26**, updated **2026-10-31**, retrieved
2026-11-17 from the [official CSV](http://www.eibispace.de/dx/sked-b26.csv).
The input has 8,971 schedule rows. Attribution: **Eike Bierwirth / EiBi**.
The official [conditions](http://www.eibispace.de/dx/README.TXT) permit use,
copying, distribution and incorporation in third-party software.

[`saturn-swl.provenance.json`](../templates/saturn-swl.provenance.json) records
the original download checksum and encoding and the bundled checksum. The
original Windows-1252 file is converted to UTF-8/LF without changing its fields.
The official HTTPS endpoint had an expired certificate when checked; the
snapshot was retrieved from its public HTTP endpoint. Runtime uses the bundled
snapshot, with no external schedule request.

Listings describe schedules, not detected stations. An outdated season is
identified in the source readout and frequency tooltips. For overnight entries,
the weekday and seasonal dates refer to the UTC day on which the entry starts.
EiBi's date fields are DDMM; bracketed last-heard dates are not validity limits.

## Delivery and bounded display work

The ordinary web-assets helper installs `saturn-swl.csv` into the release
webroot, and the existing authenticated `/remote-assets/swl.csv` route serves
only that fixed file. It does not expose arbitrary paths. The normal release
manifest covers the CSV like the other assets.

The schedule is fetched once when station labels are enabled. Streaming and
parsing enforce a 2 MB / 20,000-row limit, with a 15-second fetch timeout.
Parsing yields every 256 lines. A sorted frequency index restricts lookups to
the visible range; schedule filtering updates at UTC minute boundaries. A
failed load leaves the radio UI and band guidance available; toggling station
labels off/on retries the load.

One pending animation callback renders the newest requested view. Up to 96
frequency ticks and 24 labels occupy two collision-free lanes. Unchanged views
do not rebuild their DOM. The source CSV is a separate asset, not JavaScript
payload or IQ data. The guide occupies two slim frequency-aligned rails, including
in 3D, and station labels are hidden during transmit display suppression.

## Verification and updating

```sh
cd update_manager/remote-web
npm ci
npm run typecheck
npm run check:seam
npm test
npm run build
npm run validate:band-guide -- --output=/tmp/saturn-band-guide-review
```

The Chrome fixture loads the actual page, bundle and source CSV without a
Bridge or microphone. It checks clipping, overlapping Ham/SWL ranges, UTC
station details, bounded labels, newest-view coalescing, zero DOM mutations
over 1,000 unchanged updates, phone/3D layouts, saved options, literal rendering
of hostile CSV text and CSV failure isolation. Screenshots and a JSON report
are retained. Server tests check the asset route and its fixed allowlist;
release-manifest and worktree tests exercise the shared asset helpers.

For the next data edition, update the UTF-8/LF CSV, its provenance, `SWL_SOURCE`
season/date bounds and the source-row-count regression together, then rebuild
and run these checks. No Bridge or WDSP change is needed for a schedule refresh.
