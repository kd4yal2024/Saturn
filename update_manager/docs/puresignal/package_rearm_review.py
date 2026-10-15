"""Assemble a review archive, preserving previous frozen native inputs/evidence.

No repository, Git, service, device or deployment mutation. The output must not
exist. Run from the isolated worktree; never use this as an install operation.
"""
import hashlib
from pathlib import Path
import subprocess
import sys
import zipfile

BASE = "861f0fbc79e0e36c68a5c8ddc157e010b8be20df"
PREVIOUS_SHA = "dd46f225d6658a58d7e6973ec1e562b2001bb22ef470990674181888649981ec"
atomic_fence = "--atomic-fence" in sys.argv[1:]
scheduled_owner = "--scheduled-owner" in sys.argv[1:]
pump_stop = "--pump-stop" in sys.argv[1:]
satp_native = "--satp-native" in sys.argv[1:]
ingress_review = "--ingress" in sys.argv[1:]
settings_review = "--settings-transactions" in sys.argv[1:]
media_review = "--remote-media" in sys.argv[1:]
browser_review = "--browser-experiment" in sys.argv[1:]
cancellation_review = "--cancellation-fix" in sys.argv[1:]
managed_ui_review = "--managed-pcm-ui" in sys.argv[1:]
prequeue_review = "--settings-prequeue" in sys.argv[1:]
real_proxy_review = "--real-proxy" in sys.argv[1:]
assert sum([atomic_fence, scheduled_owner, pump_stop, satp_native, ingress_review, settings_review, media_review, browser_review, cancellation_review, managed_ui_review, prequeue_review, real_proxy_review]) <= 1
if atomic_fence:
    PREVIOUS_SHA = "a96c2ccf45c0b02f0c2bb33fce6a7234bdbb02111f670d6c623cc1903c7e80d6"
if scheduled_owner:
    PREVIOUS_SHA = "459b2730ff143ec33170670f2e9d97a2428a33825449908005aded06f540ead9"
if pump_stop:
    PREVIOUS_SHA = "1b3e2befa3e8083ff8bce29817e73d2487a65b32e2e8661a622054c28cd3aa99"
if satp_native:
    PREVIOUS_SHA = "f90dee659ba8709cf8b0edbf935774acfc691fffa005399f78c2114c1762f0c4"
if ingress_review:
    PREVIOUS_SHA = "d908152a0f358dfab6bb04213a287351f0e08bdc03d9b25ad67487a7383b531a"
if settings_review:
    PREVIOUS_SHA = "99d3402b516efedbf518fdf6f639c5dc264388e0647cd35140b5bf8edbc1cb21"
if media_review:
    PREVIOUS_SHA = "b418cfb2bec2cdad66e6092c9aa5e1ab8bd5ba5a123efe7a10963da4b8673c39"
if browser_review:
    PREVIOUS_SHA = "9dd2aabbc7e7aa1737035b52a5148c3092b249924daa2e1efccb9d914cd392b1"
if cancellation_review:
    PREVIOUS_SHA = "42122fdf306b40480d6ce272b48709ad02a29c0704803ddd61547788651da8d6"
if managed_ui_review:
    PREVIOUS_SHA = "011e9f15b770d2d201da6059ab05b72f8bc08ed592eff7e7ee4e1c699cdf1def"
if prequeue_review:
    PREVIOUS_SHA = "5bf197b905a26e347d5b229a50dfafd7130d2ddb247d36e003b4ecd3cfd14da6"
if real_proxy_review:
    PREVIOUS_SHA = "b3d11f6109e480f5ad2fba3a9450b0d01c29f015453a8d87990b24ac0438b3c7"
docs = Path(__file__).resolve().parent
root = docs.parents[2]
bridge = root / "update_manager/saturn-bridge"
previous, destination = map(Path, [arg for arg in sys.argv[1:] if arg not in {"--atomic-fence", "--scheduled-owner", "--pump-stop", "--satp-native", "--ingress", "--settings-transactions", "--remote-media", "--browser-experiment", "--cancellation-fix", "--managed-pcm-ui", "--settings-prequeue", "--real-proxy"}])
assert not destination.exists(), "refusing to overwrite an existing archive"
assert hashlib.sha256(previous.read_bytes()).hexdigest() == PREVIOUS_SHA
assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip() == BASE
with zipfile.ZipFile(previous) as archive:
    entries = {name: archive.read(name) for name in archive.namelist()}
manifest = entries.pop("SHA256SUMS").decode()
for line in manifest.splitlines():
    digest, name = line.split("  ", 1)
    assert hashlib.sha256(entries[name]).hexdigest() == digest, name
for directory in [bridge / "src", bridge / "tests", bridge / "native-stubs", docs]:
    for file in directory.rglob("*"):
        if file.is_file() and "__pycache__" not in file.parts:
            entries[file.relative_to(root).as_posix()] = file.read_bytes()
for name in ["Cargo.toml", "Cargo.lock", "build.rs"]:
    file = bridge / name
    entries[file.relative_to(root).as_posix()] = file.read_bytes()
if real_proxy_review:
    server = root / "update_manager/rust-server"
    for file in (server / "src").rglob("*"):
        if file.is_file():
            entries[file.relative_to(root).as_posix()] = file.read_bytes()
    for name in ["Cargo.toml", "Cargo.lock", "build.rs"]:
        file = server / name
        entries[file.relative_to(root).as_posix()] = file.read_bytes()
    # The five include_str! inputs are required for a self-contained source
    # rebuild. No binary, generated TLS keys, credentials or cookie state.
    for name in ["update_manager/release/fpga-image-identity-v1.json",
                 "update_manager/scripts/cleanup-saturn-logs.sh",
                 "update_manager/scripts/cleanup-saturn-backups.sh",
                 "scripts/fix-LED-power-button.sh", "scripts/setup-eth-fallback.sh"]:
        entries[name] = (root / name).read_bytes()
if media_review or browser_review or cancellation_review or managed_ui_review or prequeue_review or real_proxy_review:
    web = root / "update_manager/remote-web"
    for file in (web / "src").rglob("*"):
        if file.is_file():
            entries[file.relative_to(root).as_posix()] = file.read_bytes()
    for name in ["package.json", "package-lock.json", "tsconfig.json", "vite.config.ts", "tests/tx-generation-review.test.ts", "tests/tx-opus-encoder.test.ts", "tests/tx-uplink.test.ts"]:
        file = web / name
        entries[file.relative_to(root).as_posix()] = file.read_bytes()
    if managed_ui_review or prequeue_review or real_proxy_review:
        # Include the actual UI seam and build inputs, not just the reference
        # harness. No node_modules, deployment payload or installer invocation.
        for directory in [web / "assets", web / "styles", web / "scripts", web / "tests"]:
            for file in directory.rglob("*"):
                if file.is_file() and "__pycache__" not in file.parts:
                    entries[file.relative_to(root).as_posix()] = file.read_bytes()
        for name in ["update_manager/templates/saturn-remote-next.html", "docs/ui-redesign/migration.md"]:
            entries[name] = (root / name).read_bytes()
    if browser_review or cancellation_review or managed_ui_review or prequeue_review or real_proxy_review:
        for file in (web / "experiments").rglob("*"):
            if file.is_file() and "__pycache__" not in file.parts:
                entries[file.relative_to(root).as_posix()] = file.read_bytes()
        file = web / "tests/browser-capture-review.test.ts"
        entries[file.relative_to(root).as_posix()] = file.read_bytes()
        # Keep success AND failure. Never replace a fault with a fabricated
        # passing control or omit measured flush-tail packets from the corpus.
        artifact_roots = [
            ("comparison", "/tmp/saturn-ps-browser-release-evidence-20261013"),
            ("earlier-browser-control-P2", "/tmp/saturn-ps-browser-qualified-control-20261013-P2"),
            ("earlier-browser-control-DirectXdma", "/tmp/saturn-ps-browser-qualified-control-20261013-DirectXdma"),
            ("stale-delivery-positive-P2", "/tmp/saturn-ps-browser-final-verified-20261013-P2"),
            ("stale-delivery-failed-DirectXdma", "/tmp/saturn-ps-browser-final-verified-20261013-DirectXdma"),
            ("final-browser-P2", "/tmp/saturn-ps-browser-audit-complete-20261013-P2"),
            ("final-browser-DirectXdma", "/tmp/saturn-ps-browser-audit-complete-20261013-DirectXdma"),
        ]
        if cancellation_review:
            artifact_roots = [
                ("audit-P2", "/tmp/saturn-ps-cancel-audit-20261013-P2"),
                ("audit-DirectXdma", "/tmp/saturn-ps-cancel-audit-20261013-DirectXdma"),
                ("final-P2", "/tmp/saturn-ps-cancel-final-20261013-P2"),
                ("final-DirectXdma", "/tmp/saturn-ps-cancel-final-20261013-DirectXdma"),
            ]
        if managed_ui_review:
            artifact_roots = [
                ("final-P2", "/tmp/saturn-ps-managed-ui-final-20261013-P2"),
                ("final-DirectXdma", "/tmp/saturn-ps-managed-ui-final-20261013-DirectXdma"),
                ("audit-P2", "/tmp/saturn-ps-managed-ui-audit-20261013-P2"),
                ("audit-DirectXdma", "/tmp/saturn-ps-managed-ui-audit-20261013-DirectXdma"),
                ("failed-late-offer-P2", "/tmp/saturn-ps-managed-ui-eighth-20261013-P2"),
                ("failed-stale-P2", "/tmp/saturn-ps-managed-ui-seventh-20261013-P2"),
            ]
        if prequeue_review:
            artifact_roots = [
                ("before-fix-P2", "/tmp/saturn-ps-prequeue-reproducer-20261013-P2"),
                ("final-P2", "/tmp/saturn-ps-prequeue-final-20261013-P2"),
                ("final-DirectXdma", "/tmp/saturn-ps-prequeue-final-20261013-DirectXdma"),
            ]
        if real_proxy_review:
            artifact_roots = [
                ("basic-browser-P2", "/tmp/saturn-ps-real-proxy-first-20261013-P2"),
                ("basic-browser-DirectXdma", "/tmp/saturn-ps-real-proxy-first-20261013-DirectXdma"),
                ("earlier-cookie-P2", "/tmp/saturn-ps-real-proxy-final-20261013-P2"),
                ("earlier-cookie-DirectXdma", "/tmp/saturn-ps-real-proxy-final-20261013-DirectXdma"),
                ("final-default-cookie-P2", "/tmp/saturn-ps-real-proxy-default-entry-20261013-P2"),
                ("final-default-cookie-DirectXdma", "/tmp/saturn-ps-real-proxy-default-entry-20261013-DirectXdma"),
            ]
        for label, directory in artifact_roots:
            directory = Path(directory)
            assert directory.is_dir(), directory
            for file in directory.rglob("*"):
                if file.is_file():
                    prefix = "real-proxy" if real_proxy_review else "settings-prequeue" if prequeue_review else "managed-pcm-ui" if managed_ui_review else "cancellation-fix" if cancellation_review else "browser-experiment"
                    entries[f"{prefix}/{label}/{file.relative_to(directory).as_posix()}"] = file.read_bytes()
readme = "REAL_PROXY_PACKAGE.txt" if real_proxy_review else "SETTINGS_PREQUEUE_PACKAGE.txt" if prequeue_review else "MANAGED_PCM_UI_PACKAGE.txt" if managed_ui_review else "CANCELLATION_FIX_PACKAGE.txt" if cancellation_review else "BROWSER_EXPERIMENT_PACKAGE.txt" if browser_review else "REMOTE_MEDIA_PACKAGE.txt" if media_review else "SETTINGS_TRANSACTIONS_PACKAGE.txt" if settings_review else "INGRESS_PACKAGE.txt" if ingress_review else "SATP_NATIVE_PACKAGE.txt" if satp_native else "PUMP_STOP_PACKAGE.txt" if pump_stop else "SCHEDULED_OWNER_PACKAGE.txt" if scheduled_owner else "ATOMIC_FENCE_PACKAGE.txt" if atomic_fence else "REARM_OWNER_PACKAGE.txt"
entries["README.txt"] = (docs / readme).read_bytes()
results = [
    ("rearm-owner-native-result.txt", "/tmp/puresignal-rearm-owner-native.txt", "test result: ok. 1 passed"),
    ("rearm-owner-cargo-result.txt", "/tmp/puresignal-rearm-owner-regressions.txt", "387 passed"),
    ("rearm-default-cargo-result.txt", "/tmp/puresignal-rearm-default-regressions.txt", "377 passed"),
    ("rearm-standalone-native-result.txt", "/tmp/puresignal-rearm-native-regressions.txt", "native positive control passed"),
]
if atomic_fence:
    results = [
        ("fence-owner-native-result.txt", "/tmp/puresignal-fence-owner-native.txt", "test result: ok. 1 passed"),
        ("fence-owner-cargo-result.txt", "/tmp/puresignal-fence-owner-regressions.txt", "392 passed"),
        ("fence-default-cargo-result.txt", "/tmp/puresignal-fence-default-regressions.txt", "377 passed"),
        ("fence-standalone-native-result.txt", "/tmp/puresignal-fence-native-regressions.txt", "native positive control passed"),
    ]
if scheduled_owner:
    results = [
        ("scheduled-owner-native-result.txt", "/tmp/puresignal-scheduled-owner-native.txt", "test result: ok. 2 passed"),
        ("scheduled-owner-cargo-result.txt", "/tmp/puresignal-scheduled-owner-regressions.txt", "396 passed"),
        ("scheduled-default-cargo-result.txt", "/tmp/puresignal-scheduled-default-regressions.txt", "377 passed"),
        ("scheduled-standalone-native-result.txt", "/tmp/puresignal-scheduled-native-regressions.txt", "native positive control passed"),
    ]
if pump_stop:
    results = [
        ("pump-stop-focused-result.txt", "/tmp/puresignal-pump-stop-focused.txt", "test result: ok. 5 passed"),
        ("pump-stop-owner-native-result.txt", "/tmp/puresignal-pump-stop-owner-native.txt", "test result: ok. 2 passed"),
        ("pump-stop-owner-cargo-result.txt", "/tmp/puresignal-pump-stop-owner-regressions.txt", "401 passed"),
        ("pump-stop-default-cargo-result.txt", "/tmp/puresignal-pump-stop-default-regressions.txt", "377 passed"),
        ("pump-stop-standalone-native-result.txt", "/tmp/puresignal-pump-stop-native-regressions.txt", "native positive control passed"),
    ]
if satp_native:
    results = [
        ("satp-full-owner-native-result.txt", "/tmp/puresignal-satp-owner-native.txt", "test result: ok. 4 passed"),
        ("satp-full-owner-cargo-result.txt", "/tmp/puresignal-satp-owner-tests.txt", "408 passed"),
        ("satp-full-default-cargo-result.txt", "/tmp/puresignal-satp-default-tests.txt", "377 passed"),
        ("satp-full-standalone-native-result.txt", "/tmp/puresignal-satp-native-tests.txt", "native positive control passed"),
    ]
if ingress_review:
    results = [
        ("ingress-owner-native-result.txt", "/tmp/puresignal-ingress-owner-native.txt", "test result: ok. 5 passed"),
        ("ingress-owner-cargo-result.txt", "/tmp/puresignal-ingress-owner-tests.txt", "418 passed"),
        ("ingress-default-cargo-result.txt", "/tmp/puresignal-ingress-default-tests.txt", "379 passed"),
        ("ingress-standalone-native-result.txt", "/tmp/puresignal-ingress-native-tests.txt", "native positive control passed"),
    ]
if settings_review:
    results = [
        ("settings-focused-result.txt", "/tmp/puresignal-settings-focused.txt", "test result: ok. 9 passed"),
        ("settings-owner-native-result.txt", "/tmp/puresignal-settings-owner-native.txt", "test result: ok. 5 passed"),
        ("settings-owner-cargo-result.txt", "/tmp/puresignal-settings-owner-tests.txt", "427 passed"),
        ("settings-default-cargo-result.txt", "/tmp/puresignal-settings-default-tests.txt", "379 passed"),
        ("settings-standalone-native-result.txt", "/tmp/puresignal-settings-native-tests.txt", "native positive control passed"),
    ]
if media_review:
    results = [
        ("media-history-focused-result.txt", "/tmp/puresignal-media-history-focused.txt", "test result: ok. 8 passed"),
        ("media-ingress-focused-result.txt", "/tmp/puresignal-media-ingress-focused.txt", "test result: ok. 7 passed"),
        ("media-owner-native-result.txt", "/tmp/puresignal-media-owner-native.txt", "test result: ok. 7 passed"),
        ("media-owner-cargo-result.txt", "/tmp/puresignal-media-owner-tests.txt", "442 passed"),
        ("media-default-cargo-result.txt", "/tmp/puresignal-media-default-tests.txt", "379 passed"),
        ("media-standalone-native-result.txt", "/tmp/puresignal-media-native-tests.txt", "native positive control passed"),
        ("media-client-focused-result.txt", "/tmp/puresignal-media-client-tests.txt", "38 passed"),
        ("media-client-full-worktree-result.txt", "/tmp/puresignal-media-client-all-tests.txt", "711 passed"),
        ("media-client-typecheck-result.txt", "/tmp/puresignal-media-client-typecheck.txt", "tsc --noEmit"),
    ]
if browser_review:
    results = [
        ("browser-cancellation-reproducer.txt", "/tmp/puresignal-browser-cancel-reproducer.txt", "1 passed"),
        ("browser-owner-stubs.txt", "/tmp/puresignal-browser-owner-final.txt", "443 passed"),
        ("browser-decoder-corpus.txt", "/tmp/puresignal-browser-release-decoder.txt", "saved audio matched"),
        ("browser-native-regressions.txt", "/tmp/puresignal-browser-native-regressions.txt", "7 passed"),
        ("browser-earlier-actual-native-control.txt", "/tmp/puresignal-browser-final-native.txt", "1 passed"),
        ("browser-actual-native-failure.txt", "/tmp/puresignal-browser-final-verified.txt", "settings_not_applied"),
        ("browser-final-actual-native-control.txt", "/tmp/puresignal-browser-audit-complete.txt", "1 passed"),
        ("browser-targeted-web.txt", "/tmp/puresignal-browser-web-tests.txt", "41 passed"),
        ("browser-full-web.txt", "/tmp/puresignal-browser-full-web.txt", "714 passed"),
        ("browser-typecheck.txt", "/tmp/puresignal-browser-typecheck.txt", "tsc --noEmit"),
        ("browser-comparison.txt", "/tmp/puresignal-browser-release-comparison.txt", "Browser experiment retained"),
    ]
if cancellation_review:
    results = [
        ("cancellation-owner-stubs.txt", "/tmp/puresignal-cancel-owner-final.txt", "451 passed"),
        ("cancellation-default-stubs.txt", "/tmp/puresignal-cancel-default-final.txt", "379 passed"),
        ("cancellation-combined-native.txt", "/tmp/puresignal-cancel-combined-native.txt", "7 passed"),
        ("cancellation-standalone-native.txt", "/tmp/puresignal-cancel-standalone-final.txt", "native positive control passed"),
        ("cancellation-browser-audit.txt", "/tmp/puresignal-cancel-browser-audit.txt", "1 passed"),
        ("cancellation-browser-earlier-pass.txt", "/tmp/puresignal-cancel-browser-final.txt", "1 passed"),
        ("cancellation-web-tests.txt", "/tmp/puresignal-cancel-web-tests.txt", "714 passed"),
        ("cancellation-web-typecheck.txt", "/tmp/puresignal-cancel-web-typecheck.txt", "tsc --noEmit"),
        ("cancellation-experiment-typecheck.txt", "/tmp/puresignal-cancel-experiment-typecheck.txt", ""),
    ]
if managed_ui_review:
    results = [
        ("managed-pcm-owner-stubs.txt", "/tmp/puresignal-managed-pcm-owner-final.txt", "452 passed"),
        ("managed-pcm-default-stubs.txt", "/tmp/puresignal-managed-pcm-default-final.txt", "379 passed"),
        ("managed-pcm-combined-native.txt", "/tmp/puresignal-managed-pcm-combined-native.txt", "7 passed"),
        ("managed-pcm-standalone-native.txt", "/tmp/puresignal-managed-pcm-standalone-native.txt", "native positive control passed"),
        ("managed-pcm-ui-final.txt", "/tmp/puresignal-managed-pcm-ui-final.txt", "1 passed"),
        ("managed-pcm-ui-audit.txt", "/tmp/puresignal-managed-pcm-ui-audit.txt", "1 passed"),
        ("managed-pcm-ui-failed-late-offer.txt", "/tmp/puresignal-managed-pcm-ui-eighth.txt", "test result: FAILED"),
        ("managed-pcm-ui-failed-stale.txt", "/tmp/puresignal-managed-pcm-ui-seventh.txt", "Some(Stale)"),
        ("managed-pcm-web-tests.txt", "/tmp/puresignal-managed-pcm-web-final.txt", "736 passed"),
        ("managed-pcm-typecheck.txt", "/tmp/puresignal-managed-pcm-typecheck.txt", "tsc --noEmit"),
        ("managed-pcm-seam.txt", "/tmp/puresignal-managed-pcm-seam-final.txt", "check-template-seam: OK"),
        ("managed-pcm-migration.txt", "/tmp/puresignal-managed-pcm-migration-final.txt", "generate-migration-doc: OK"),
        ("managed-pcm-build.txt", "/tmp/puresignal-managed-pcm-build.txt", "in one IIFE"),
    ]
if prequeue_review:
    results = [
        ("prequeue-before-web.txt", "/tmp/puresignal-prequeue-before-web.txt", "3 failed | 25 passed"),
        ("prequeue-before-browser.txt", "/tmp/puresignal-prequeue-reproducer-browser.txt", "test result: FAILED"),
        ("prequeue-tooling-error.txt", "/tmp/puresignal-prequeue-before-browser.txt", "Python 3.10"),
        ("prequeue-final-browser.txt", "/tmp/puresignal-prequeue-final-browser.txt", "1 passed"),
        ("prequeue-owner-stubs.txt", "/tmp/puresignal-prequeue-owner.txt", "452 passed"),
        ("prequeue-web-tests.txt", "/tmp/puresignal-prequeue-web.txt", "742 passed"),
        ("prequeue-typecheck.txt", "/tmp/puresignal-prequeue-typecheck.txt", "tsc --noEmit"),
        ("prequeue-build.txt", "/tmp/puresignal-prequeue-build.txt", "in one IIFE"),
        ("prequeue-seam.txt", "/tmp/puresignal-prequeue-seam.txt", "check-template-seam: OK"),
        ("prequeue-migration.txt", "/tmp/puresignal-prequeue-migration.txt", "generate-migration-doc: OK"),
    ]
if real_proxy_review:
    results = [
        ("real-proxy-executable-build.txt", "/tmp/puresignal-real-proxy-build.txt", "Finished `dev` profile"),
        ("real-proxy-server-unit.txt", "/tmp/puresignal-real-proxy-unit.txt", "34 passed"),
        ("real-proxy-server-all.txt", "/tmp/puresignal-real-proxy-server-all.txt", "154 passed"),
        ("real-proxy-basic-browser.txt", "/tmp/puresignal-real-proxy-first.txt", "1 passed"),
        ("real-proxy-cookie-browser.txt", "/tmp/puresignal-real-proxy-final.txt", "1 passed"),
        ("real-proxy-default-entry-browser.txt", "/tmp/puresignal-real-proxy-default-entry.txt", "1 passed"),
        ("real-proxy-web-tests.txt", "/tmp/puresignal-real-proxy-web.txt", "742 passed"),
        ("real-proxy-web-typecheck.txt", "/tmp/puresignal-real-proxy-web-typecheck.txt", "tsc --noEmit"),
        ("real-proxy-extracted-missing-identity.txt", "/tmp/puresignal-real-proxy-extracted-server.txt", "153 passed; 1 failed"),
        ("real-proxy-extracted-explicit-identity.txt", "/tmp/puresignal-real-proxy-extracted-identity.txt", "154 passed"),
    ]
for archive_name, source, expected in results:
    data = Path(source).read_bytes()
    assert expected.encode() in data, source
    entries["offline-native/" + archive_name] = data
for name, paths in [
    ("cargo-offline-policy-feature.patch", ["update_manager/saturn-bridge/Cargo.toml"]),
    ("main-native-linkage.patch", ["update_manager/saturn-bridge/src/main.rs"]),
    ("owner-output-boundaries.patch", ["update_manager/saturn-bridge/src/p2/session.rs", "update_manager/saturn-bridge/src/xdma_tx_radio.rs", "update_manager/saturn-bridge/src/tx_monitor.rs", "update_manager/saturn-bridge/src/tx_thread.rs", "update_manager/saturn-bridge/src/tci/mod.rs", "update_manager/saturn-bridge/src/tci/client.rs", "update_manager/saturn-bridge/src/tci/outbound.rs", "update_manager/saturn-bridge/src/satp.rs", "update_manager/saturn-bridge/src/tx_audio.rs", "update_manager/saturn-bridge/src/wdsp.rs"]),
]:
    entries[name] = subprocess.check_output(["git", "diff", BASE, "--", *paths], cwd=root)
entries["SHA256SUMS"] = "".join(
    hashlib.sha256(data).hexdigest() + "  " + name + "\n"
    for name, data in sorted(entries.items())
).encode()
with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED) as archive:
    for name, data in sorted(entries.items()):
        archive.writestr(name, data)
with zipfile.ZipFile(destination) as archive:
    assert archive.testzip() is None
print(destination)
print("entries:", len(entries), "bytes:", destination.stat().st_size)
print("sha256:", hashlib.sha256(destination.read_bytes()).hexdigest())
