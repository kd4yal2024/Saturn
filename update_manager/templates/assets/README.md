# Saturn Go Local Web Assets

These files are deployed under `/var/lib/saturn-web/assets` so the appliance
management UI and Saturn Remote remain functional without public internet
access.

Vendored components:

- `vendor/tailwind.js`: Tailwind CSS browser runtime 3.4.17, MIT license,
  originally distributed from `https://cdn.tailwindcss.com/`.
- `vendor/chart.js`: Chart.js 4.5.1, MIT license.
- `vendor/ansi_up.min.js`: ansi_up 5.2.1, MIT license.
- `fonts/inter-latin-var.woff2`: Inter variable Latin font, SIL Open Font
  License 1.1.

The JavaScript files are intentionally pinned in the repository. Update them
only as a reviewed dependency change, record the version here, and run the
template syntax, Rust route, and offline asset checks before deployment.

`tailwind.js` is retained as a compatibility bridge for the existing utility
class templates. A future build-time CSS pass can replace the browser runtime
after every template has migrated to the shared component stylesheet.
