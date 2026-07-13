#!/usr/bin/env python3
# update-pihpsdr.py - piHPSDR Update Script
# Automates cloning, updating, and building the pihpsdr repository
# Version: 1.12
# Written by: Jerry DeLong KD4YAL
# Changes: Removed --show-compile flag, merged into --verbose, fixed make process output to display in CLI,
#          changed make output color to white in CLI with --verbose, compacted noisy dependency output,
#          added WDSP 2.00 Linux compatibility and dependency preflight
# Dependencies: psutil (version 7.0.0) in ~/venv, optional pyfiglet, urllib.error
# Usage: python3 /opt/saturn-go/scripts/update-pihpsdr.py

import os
import sys
import time
import subprocess
import shutil
import glob
import argparse
import logging
import re
from datetime import datetime
from pathlib import Path
import psutil
import urllib.error

sys.dont_write_bytecode = True

try:
    from pyfiglet import Figlet
except ImportError:
    Figlet = None  # Handle missing pyfiglet

# ANSI color codes
class Colors:
    RED = '\033[31m'    # Standard red for banner
    BLUE = '\033[34m'   # Standard blue for subtitle
    CYAN = '\033[36m'   # Cyan for info messages
    GREEN = '\033[32m'  # Green for success
    YELLOW = '\033[33m' # Yellow for warnings
    WHITE = '\033[37m'  # White for build output
    END = '\033[0m'

def _is_subpath(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False

def guard_repo_tree_python_execution():
    script_path = Path(__file__).resolve()
    candidate_roots = []
    for candidate in (
        os.environ.get("SATURN_REPO_ROOT"),
        os.environ.get("SATURN_DIR"),
        str(Path.home() / "github" / "Saturn"),
    ):
        if not candidate:
            continue
        root = Path(candidate).expanduser().resolve()
        if root not in candidate_roots:
            candidate_roots.append(root)

    for root in candidate_roots:
        if _is_subpath(script_path, root):
            print(
                f"{Colors.RED}✗ Refusing to run Python updater from repo tree: {script_path}\n"
                f"Use installed script: /opt/saturn-go/scripts/{script_path.name}{Colors.END}",
                file=sys.stderr,
            )
            sys.exit(2)

# Script metadata
SCRIPT_NAME = "piHPSDR Update"
SCRIPT_VERSION = "1.12"
SCRIPT_START_TIME = datetime.now()
TIMESTAMP = SCRIPT_START_TIME.strftime('%Y%m%d-%H%M%S')
PIHPSDR_DIR = Path.home() / "github" / "pihpsdr"
LOG_DIR = Path.home() / "saturn-logs"
BACKUP_DIR = Path.home() / f"pihpsdr-backup-{TIMESTAMP}"
REPO_URL = "https://github.com/dl1ycf/pihpsdr"
DEFAULT_BRANCH = "master"
TMP_DIR = Path("/tmp") / f"pihpsdr-update-{TIMESTAMP}-{os.getpid()}"

# Terminal utilities
def get_term_size():
    try:
        cols = os.get_terminal_size().columns
        lines = os.get_terminal_size().lines
    except OSError:
        cols, lines = 80, 24
    cols = max(40, min(cols, 80))
    lines = max(15, lines)
    return cols, lines

def truncate_text(text, max_len):
    clean_text = ''.join(c for c in text if c.isprintable())
    if len(clean_text) > max_len:
        return clean_text[:max_len-2] + ".."
    return clean_text

def debug_print(msg):
    if args.debug:
        print(f"{Colors.END}[DEBUG] {msg}{Colors.END}")
        logging.debug(msg)

def temp_log_path(name):
    return TMP_DIR / f"{name}.log"

# UI functions
def print_header(title):
    cols, _ = get_term_size()
    title = truncate_text(title, cols-12)
    print(f"\n{Colors.BLUE}═════ {title} ═════{Colors.END}\n")
    logging.info(f"Header: {title}")

def print_success(msg):
    print(f"{Colors.GREEN}✔ {msg}{Colors.END}")
    logging.info(f"Success: {msg}")

def print_warning(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols-7)
    print(f"{Colors.YELLOW}⚠ {msg}{Colors.END}")
    logging.warning(msg)

def print_error(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols-7)
    print(f"{Colors.RED}✗ {msg}{Colors.END}", file=sys.stderr)
    logging.error(msg)
    sys.exit(1)

def print_info(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols-7)
    print(f"{Colors.CYAN}ℹ {msg}{Colors.END}")
    logging.info(msg)

def print_build_output(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols-7)
    # Keep compile output uncolored so web UI light theme remains readable.
    print(msg)
    logging.info(msg)

class DependencyOutputFilter:
    """Compact routine apt/debconf chatter while preserving build/install signal."""
    def __init__(self):
        self.in_autoremove_block = False
        self.suppressed = 0
        self.saw_autoremove_notice = False

    def filter(self, line):
        text = line.strip()
        if not text:
            return None

        if self.in_autoremove_block:
            self.suppressed += 1
            if text.startswith("Use 'sudo apt autoremove'"):
                self.in_autoremove_block = False
            return None

        if text.startswith("The following packages were automatically installed and are no longer required"):
            self.in_autoremove_block = True
            self.saw_autoremove_notice = True
            self.suppressed += 1
            return None

        if text.startswith("debconf:"):
            self.suppressed += 1
            return None

        if text.startswith("WARNING: apt does not have a stable CLI interface"):
            self.suppressed += 1
            return None

        if text in (
            "Reading package lists...",
            "Building dependency tree...",
            "Reading state information...",
        ):
            self.suppressed += 1
            return None

        if re.fullmatch(
            r"0 upgraded, 0 newly installed, 0 to remove and \d+ not upgraded\.",
            text,
        ):
            self.suppressed += 1
            return None

        if text == "rm: cannot remove 'SoapySDR': No such file or directory":
            self.suppressed += 1
            return None

        return text

def dependency_env():
    env = os.environ.copy()
    env["DEBIAN_FRONTEND"] = "noninteractive"
    env["APT_LISTCHANGES_FRONTEND"] = "none"
    env["NEEDRESTART_MODE"] = "a"
    return env

def stream_process_output(process, output_filter=None):
    for raw_line in process.stdout:
        line = raw_line.rstrip("\n")
        if output_filter is not None:
            line = output_filter.filter(line)
        elif not line.strip():
            line = None
        if line:
            print_build_output(line)
    return process.wait()

def progress_bar(pid, msg, total_steps):
    if args.dry_run:
        print_info(f"[Dry Run] Simulating progress for: {msg}")
        return 0
    cols, _ = get_term_size()
    max_width = cols - 20
    msg = truncate_text(msg, max_width)
    print(f"{Colors.CYAN}Progress: {msg}{Colors.END}")
    return pid.wait()

# Initialize logging
def init_logging(verbose=False):
    debug_print("Initializing logging")
    try:
        LOG_DIR.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        print_error(f"Failed to create log dir: {str(e)}")
    try:
        TMP_DIR.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        print_error(f"Failed to create temp dir: {str(e)}")
    class Tee:
        def __init__(self, *files):
            self.files = files
        def write(self, data):
            for f in self.files:
                try:
                    f.write(data)
                except UnicodeEncodeError:
                    # Some invokers expose latin-1 stdout/stderr; replace unsupported
                    # symbols only for that stream instead of crashing the update.
                    encoding = getattr(f, "encoding", None) or "utf-8"
                    safe_data = data.encode(encoding, errors="replace").decode(encoding, errors="replace")
                    f.write(safe_data)
                f.flush()
        def flush(self):
            for f in self.files:
                f.flush()
    log_file = LOG_DIR / f"pihpsdr-update-{TIMESTAMP}.log"
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        handlers=[logging.FileHandler(log_file, encoding="utf-8")]
    )
    try:
        log_handle = open(log_file, 'a', encoding="utf-8")
    except Exception as e:
        print_error(f"Failed to open log file {log_file}: {str(e)}")
    sys.stdout = Tee(sys.__stdout__, log_handle)
    sys.stderr = Tee(sys.__stderr__, log_handle)
    if sys.__stdout__.isatty() and shutil.which("tput"):
        subprocess.run(
            ["tput", "clear"],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    cols, _ = get_term_size()
    if Figlet:
        f = Figlet(font='standard', width=cols-2, justify='center')
        pihpsdr_ascii = f.renderText('piHPSDR')
    else:
        pihpsdr_ascii = "piHPSDR\n"
    banner = f"""
{Colors.RED}{pihpsdr_ascii.rstrip()}{Colors.END}
{Colors.BLUE}{f'Update Manager v{SCRIPT_VERSION}'.center(cols-2)}{Colors.END}\n\n"""
    logging.debug(f"Banner raw output: {repr(banner)}")
    print(banner)
    print_info(f"Started: {SCRIPT_START_TIME}")
    print_info(f"Log: {log_file}")

# Parse command-line arguments
def parse_args():
    parser = argparse.ArgumentParser(description="piHPSDR Update Script")
    parser.add_argument("--skip-git", action="store_true", help="Skip Git repository update")
    parser.add_argument("-y", "--yes", action="store_true", help="Auto-confirm backup creation")
    parser.add_argument("-n", "--no", action="store_true", help="Skip backup creation")
    parser.add_argument("--no-gpio", action="store_true", help="Disable GPIO for Radioberry")
    parser.add_argument("--dry-run", action="store_true", help="Simulate actions without executing")
    parser.add_argument("--verbose", action="store_true", help="Enable verbose output for all commands, including detailed compile output")
    parser.add_argument("--debug", action="store_true", help="Enable debug output")
    args = parser.parse_args()
    if args.yes and args.no:
        print_error("Cannot use -y and -n together")
    return args

# Check system requirements
def check_requirements():
    debug_print("Checking requirements")
    print_header("System Check")
    print(f"{Colors.CYAN}⚡ Verifying requirements...{Colors.END}")
    requirements = ["git", "make", "gcc", "sudo", "rsync"]
    for item in requirements:
        print(f"{Colors.CYAN}Scanning: {item}...{Colors.END}")
        time.sleep(0.3)
        if shutil.which(item):
            cols, _ = get_term_size()
            print(f"{Colors.GREEN}✓ {item} - OK{' ' * (cols - len(item) - 8)}{Colors.END}")
        else:
            print_error(f"Missing command: {item}")
    print(f"\n{Colors.GREEN}[SCAN COMPLETE]{Colors.END}\n")
    try:
        free_space = psutil.disk_usage(str(Path.home())).free / 1024**3  # Convert to GB
        cols, _ = get_term_size()
        if free_space < 1:
            print_warning(f"Low disk space: {free_space:.2f}GB")
        else:
            print_success(f"Disk: {free_space:.2f}GB free")
        print_success("Requirements met")
    except Exception as e:
        print_error(f"Failed to check disk space: {str(e)}")

# Check connectivity
def check_connectivity():
    debug_print("Checking connectivity")
    if args.skip_git:
        print_warning("Skipping network check")
        return 0
    print_header("Network Check")
    print(f"{Colors.CYAN}⚡ Checking connectivity...{Colors.END}")
    max_attempts = 3
    for attempt in range(1, max_attempts + 1):
        try:
            result = subprocess.run(["ping", "-c", "1", "-W", "2", "github.com"], check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            rtt_match = re.search(r"time=([\d.]+)\s*ms", result.stdout)
            rtt = float(rtt_match.group(1)) if rtt_match else None
            if args.verbose:
                print_info(f"RTT: {rtt:.3f} ms")
            print_success("Network verified")
            return 0
        except subprocess.CalledProcessError as e:
            if attempt < max_attempts:
                print_warning(f"Cannot reach GitHub (attempt {attempt}/{max_attempts}): {e.output.strip()}. Retrying...")
                time.sleep(2)
            else:
                print_warning(f"Cannot reach GitHub after {max_attempts} attempts: {e.output.strip()}")
                return 1

# Create backup
def create_backup():
    debug_print("Creating backup")
    print_header("Backup")
    cols, _ = get_term_size()
    do_backup = False
    if args.no:
        print_warning("Backup skipped via -n flag")
        return False
    if args.yes:
        do_backup = True
        print_info("Auto-creating backup via -y flag")
    elif args.dry_run:
        print_info("[Dry Run] Simulating backup creation")
        return True
    else:
        if not sys.__stdin__.isatty():
            print_warning("Non-interactive session: backup skipped (use -y to force)")
            return False
        print(f"{Colors.YELLOW}⚠ Backup? Y/n: {Colors.END}", end="", flush=True)
        try:
            reply = input("").lower()
        except Exception:
            reply = "n"
        print(Colors.END)
        if reply == "n":
            print_warning("Backup skipped")
            return False
        do_backup = True
    if do_backup:
        print(f"{Colors.CYAN}⚡ Creating backup...{Colors.END}")
        backup_pattern = str(Path.home() / "pihpsdr-backup-*")
        backup_dirs = sorted(glob.glob(backup_pattern), key=os.path.getmtime, reverse=True)
        print_info(f"Found {len(backup_dirs)} existing backups")
        if len(backup_dirs) > 2:
            for old_backup in backup_dirs[2:]:
                try:
                    if not args.dry_run:
                        shutil.rmtree(old_backup)
                    print_info(f"Deleted old backup: {old_backup}")
                except Exception as e:
                    print_warning(f"Failed to delete backup {old_backup}: {str(e)}")
        print_info(f"Location: {BACKUP_DIR}")
        try:
            if not args.dry_run:
                BACKUP_DIR.mkdir(parents=True, exist_ok=True)
        except Exception as e:
            print_error(f"Cannot create backup dir: {str(e)}")
        rsync_log = temp_log_path("rsync_output")
        try:
            with rsync_log.open("w") as f:
                process = subprocess.Popen(["rsync", "-a", f"{PIHPSDR_DIR}/", str(BACKUP_DIR)], stdout=f, stderr=f, text=True)
                return_code = progress_bar(process, "Copying files", 50)
                if return_code != 0:
                    error_output = rsync_log.read_text(errors="replace").strip()
                    print_error(f"Backup failed: {error_output}")
                if args.verbose:
                    print_info(f"Rsync output: {rsync_log.read_text(errors='replace').strip()}")
        except Exception as e:
            print_error(f"Failed to access {rsync_log}: {str(e)}")
        if args.dry_run:
            print_info("[Dry Run] Backup created")
            return True
        backup_size = sum(f.stat().st_size for f in BACKUP_DIR.rglob('*') if f.is_file()) / 1024**2
        print_info(f"Size: {backup_size:.1f}MB")
        print_success("Backup created")
        return True
    return False

SATURN_PIHPSDR_PATCH_MARKER = "SATURN_PS_SETPK_PCBV3"
SATURN_WDSP2_COMPAT_MARKER = "SATURN_WDSP2_THREAD_NAME_COMPAT"

WDSP2_BROKEN_THREAD_NAME_BLOCK = """\t} else\tif (start_address == &doPSCalcCorrection
\t\t\t || start_address == &doPSTurnoff
\t\t || start_address == &PSSaveCorrection
\t\t || start_address == &PSRestoreCorrection) {
\t  snprintf(tname, sizeof(tname), "PURESIGNAL");"""

WDSP2_COMPAT_THREAD_NAME_BLOCK = """\t} else if (start_address == &doPSCorrChange) { // SATURN_WDSP2_THREAD_NAME_COMPAT
\t  snprintf(tname, sizeof(tname), "PURESIGNAL");"""

WDSP2_UPSTREAM_THREAD_NAME_BLOCK = """\t} else if (start_address == &doPSCorrChange) {
\t  snprintf(tname, sizeof(tname), "PURESIGNAL");"""

WDSP2_BROKEN_HANDLE_TYPE = "#define HANDLE int *"
WDSP2_COMPAT_HANDLE_TYPE = "#define HANDLE void * // SATURN_WDSP2_HANDLE_COMPAT"
WDSP2_UPSTREAM_HANDLE_TYPE = "#define HANDLE void *"

WDSP2_BROKEN_WAIT_DECL = "int LinuxWaitForMultipleObjects(int num, sem_t **sem, int waitall, int ms);"
WDSP2_COMPAT_WAIT_DECL = "int LinuxWaitForMultipleObjects(int num, void **sem, int waitall, int ms); // SATURN_WDSP2_WAIT_COMPAT"
WDSP2_UPSTREAM_WAIT_DECL = "int LinuxWaitForMultipleObjects(int num, void **sem, int waitall, int ms);"
WDSP2_BROKEN_WAIT_IMPL = """int LinuxWaitForMultipleObjects(int num, sem_t **sem, int waitall, int ms) {
  if (!waitall && ms == INFINITE) {"""
WDSP2_COMPAT_WAIT_IMPL = """int LinuxWaitForMultipleObjects(int num, void **sem, int waitall, int ms) { // SATURN_WDSP2_WAIT_COMPAT
  if (!waitall && ms == INFINITE) {"""
WDSP2_UPSTREAM_WAIT_IMPL = """int LinuxWaitForMultipleObjects(int num, void **sem, int waitall, int ms) {
  if (!waitall && ms == INFINITE) {"""
WDSP2_BROKEN_WAIT_CALL = "if (sem_trywait(sem[i]) == 0) { return i; }"
WDSP2_COMPAT_WAIT_CALL = "if (sem_trywait((sem_t *)sem[i]) == 0) { return i; } // SATURN_WDSP2_WAIT_COMPAT"
WDSP2_UPSTREAM_WAIT_CALL = "if (sem_trywait((sem_t *)sem[i]) == 0) { return i; }"

def patch_file_replace(path, old, new):
    content = path.read_text()
    if new in content:
        return False
    if old not in content:
        raise ValueError(f"expected text not found in {path}")
    path.write_text(content.replace(old, new, 1))
    return True

def revert_file_replace(path, patched, original):
    content = path.read_text()
    if patched not in content:
        return False
    path.write_text(content.replace(patched, original, 1))
    return True

def revert_saturn_pihpsdr_v3_patch():
    """Remove our generated Saturn V3 patch before pulling upstream."""
    if not PIHPSDR_DIR.exists():
        return
    transmitter = PIHPSDR_DIR / "src" / "transmitter.c"
    saturnmain = PIHPSDR_DIR / "src" / "saturnmain.c"
    if not transmitter.exists() or not saturnmain.exists():
        return

    try:
        transmitter_content = transmitter.read_text()
        if SATURN_PIHPSDR_PATCH_MARKER in transmitter_content:
            patched_include = '#include "saturndrivers.h"\n#include "sintab.h"'
            original_include = '#include "sintab.h"'
            patched_defs = """#define SATURN_PS_SETPK_LEGACY 0.6121
#define SATURN_PS_SETPK_PCBV3 0.8031

static bool saturn_uses_pcbv3_puresignal_level(void) {
  return device == NEW_DEVICE_SATURN && Saturn_PCB_Version >= 3;
}
"""
            patched_default = """      tx->ps_setpk = saturn_uses_pcbv3_puresignal_level()
                     ? SATURN_PS_SETPK_PCBV3
                     : SATURN_PS_SETPK_LEGACY;"""
            original_default = "      tx->ps_setpk = 0.6121;"
            patched_migration = """  if (saturn_uses_pcbv3_puresignal_level() && fabs(tx->ps_setpk - SATURN_PS_SETPK_LEGACY) < 0.00005) {
    tx->ps_setpk = SATURN_PS_SETPK_PCBV3;
  }
"""
            transmitter_content = transmitter_content.replace(patched_include, original_include, 1)
            transmitter_content = transmitter_content.replace(patched_defs, "", 1)
            transmitter_content = transmitter_content.replace(patched_default, original_default, 1)
            transmitter_content = transmitter_content.replace(patched_migration, "", 1)
            transmitter.write_text(transmitter_content)

        patched_scale = """//#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002A00  // 18 bit scale value - was 5/64 of full scale for PCB V3
#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002000  // V3 reverted to V2 level for PureSignal compatibility"""
        original_scale = "#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002A00  // 18 bit scale value - set to 1/32 of full scale for PCB V3"
        revert_file_replace(saturnmain, patched_scale, original_scale)
    except Exception as e:
        print_warning(f"Could not remove previous Saturn piHPSDR V3 patch before git update: {e}")

def revert_saturn_wdsp2_linux_compatibility():
    """Remove only Saturn's marked WDSP2 Linux patches before git pull."""
    linux_port = PIHPSDR_DIR / "wdsp" / "linux_port.c"
    linux_header = PIHPSDR_DIR / "wdsp" / "linux_port.h"
    try:
        if linux_port.exists():
            revert_file_replace(
                linux_port,
                WDSP2_COMPAT_THREAD_NAME_BLOCK,
                WDSP2_BROKEN_THREAD_NAME_BLOCK,
            )
            revert_file_replace(
                linux_port,
                WDSP2_COMPAT_WAIT_IMPL,
                WDSP2_BROKEN_WAIT_IMPL,
            )
            revert_file_replace(
                linux_port,
                WDSP2_COMPAT_WAIT_CALL,
                WDSP2_BROKEN_WAIT_CALL,
            )
        if linux_header.exists():
            revert_file_replace(
                linux_header,
                WDSP2_COMPAT_HANDLE_TYPE,
                WDSP2_BROKEN_HANDLE_TYPE,
            )
            revert_file_replace(
                linux_header,
                WDSP2_COMPAT_WAIT_DECL,
                WDSP2_BROKEN_WAIT_DECL,
            )
    except Exception as e:
        print_warning(f"Could not remove previous WDSP2 compatibility patch: {e}")

def apply_saturn_wdsp2_linux_compatibility():
    """Apply piHPSDR's missing WDSP2 Linux-port compatibility updates."""
    if args.dry_run:
        print_info("[Dry Run] Would apply WDSP2 Linux compatibility")
        return

    linux_port = PIHPSDR_DIR / "wdsp" / "linux_port.c"
    linux_header = PIHPSDR_DIR / "wdsp" / "linux_port.h"
    if not linux_port.exists() or not linux_header.exists():
        print_warning("Skipping WDSP2 compatibility; Linux port sources not found")
        return

    try:
        changed = False
        content = linux_port.read_text()
        if (
            WDSP2_COMPAT_THREAD_NAME_BLOCK not in content
            and WDSP2_UPSTREAM_THREAD_NAME_BLOCK not in content
        ):
            if WDSP2_BROKEN_THREAD_NAME_BLOCK in content:
                changed |= patch_file_replace(
                    linux_port,
                    WDSP2_BROKEN_THREAD_NAME_BLOCK,
                    WDSP2_COMPAT_THREAD_NAME_BLOCK,
                )
            else:
                print_warning("WDSP2 thread-name code changed upstream; no thread patch applied")

        content = linux_port.read_text()
        wait_impl_present = (
            WDSP2_COMPAT_WAIT_IMPL in content
            or WDSP2_UPSTREAM_WAIT_IMPL in content
        )
        wait_call_present = (
            WDSP2_COMPAT_WAIT_CALL in content
            or WDSP2_UPSTREAM_WAIT_CALL in content
        )
        if not wait_impl_present or not wait_call_present:
            if WDSP2_BROKEN_WAIT_IMPL in content and WDSP2_BROKEN_WAIT_CALL in content:
                changed |= patch_file_replace(
                    linux_port,
                    WDSP2_BROKEN_WAIT_IMPL,
                    WDSP2_COMPAT_WAIT_IMPL,
                )
                changed |= patch_file_replace(
                    linux_port,
                    WDSP2_BROKEN_WAIT_CALL,
                    WDSP2_COMPAT_WAIT_CALL,
                )
            else:
                print_warning("WDSP2 multi-wait code changed upstream; no wait patch applied")

        header_content = linux_header.read_text()
        if (
            WDSP2_COMPAT_HANDLE_TYPE not in header_content
            and WDSP2_UPSTREAM_HANDLE_TYPE not in header_content
        ):
            if WDSP2_BROKEN_HANDLE_TYPE in header_content:
                changed |= patch_file_replace(
                    linux_header,
                    WDSP2_BROKEN_HANDLE_TYPE,
                    WDSP2_COMPAT_HANDLE_TYPE,
                )
            else:
                print_warning("WDSP2 HANDLE type changed upstream; no handle patch applied")

        header_content = linux_header.read_text()
        if (
            WDSP2_COMPAT_WAIT_DECL not in header_content
            and WDSP2_UPSTREAM_WAIT_DECL not in header_content
        ):
            if WDSP2_BROKEN_WAIT_DECL in header_content:
                changed |= patch_file_replace(
                    linux_header,
                    WDSP2_BROKEN_WAIT_DECL,
                    WDSP2_COMPAT_WAIT_DECL,
                )
            else:
                print_warning("WDSP2 multi-wait declaration changed upstream; no declaration patch applied")

        if changed:
            print_success("Applied WDSP2 Linux compatibility")
        elif "SATURN_WDSP2_" in content or "SATURN_WDSP2_" in header_content:
            print_info("WDSP2 Linux compatibility already present")
        else:
            print_info("WDSP2 Linux compatibility is fixed upstream")
    except Exception as e:
        print_error(f"Failed to apply WDSP2 Linux compatibility: {e}")

def apply_saturn_pihpsdr_v3_patch():
    """Apply Saturn V3 PureSignal defaults to upstream piHPSDR checkout."""
    if args.dry_run:
        print_info("[Dry Run] Would apply Saturn V3 PureSignal piHPSDR patch")
        return

    transmitter = PIHPSDR_DIR / "src" / "transmitter.c"
    saturnmain = PIHPSDR_DIR / "src" / "saturnmain.c"
    if not transmitter.exists() or not saturnmain.exists():
        print_warning("Skipping Saturn V3 PureSignal patch; piHPSDR source files not found")
        return

    changed = False
    try:
        changed |= patch_file_replace(
            saturnmain,
            "#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002A00  // 18 bit scale value - set to 1/32 of full scale for PCB V3",
            """//#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002A00  // 18 bit scale value - was 5/64 of full scale for PCB V3
#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002000  // V3 reverted to V2 level for PureSignal compatibility""",
        )
        changed |= patch_file_replace(
            transmitter,
            '#include "receiver.h"\n#include "sintab.h"',
            '#include "receiver.h"\n#include "saturndrivers.h"\n#include "sintab.h"',
        )
        changed |= patch_file_replace(
            transmitter,
            "#define min(x,y) (x<y?x:y)\n#define max(x,y) (x<y?y:x)\n",
            """#define min(x,y) (x<y?x:y)
#define max(x,y) (x<y?y:x)

#define SATURN_PS_SETPK_LEGACY 0.6121
#define SATURN_PS_SETPK_PCBV3 0.8031

static bool saturn_uses_pcbv3_puresignal_level(void) {
  return device == NEW_DEVICE_SATURN && Saturn_PCB_Version >= 3;
}
""",
        )
        changed |= patch_file_replace(
            transmitter,
            "      tx->ps_setpk = 0.6121;",
            """      tx->ps_setpk = saturn_uses_pcbv3_puresignal_level()
                     ? SATURN_PS_SETPK_PCBV3
                     : SATURN_PS_SETPK_LEGACY;""",
        )
        changed |= patch_file_replace(
            transmitter,
            "  tx_restore_state(tx);\n",
            """  tx_restore_state(tx);
  if (saturn_uses_pcbv3_puresignal_level() && fabs(tx->ps_setpk - SATURN_PS_SETPK_LEGACY) < 0.00005) {
    tx->ps_setpk = SATURN_PS_SETPK_PCBV3;
  }
""",
        )
    except Exception as e:
        print_error(f"Failed to apply Saturn V3 PureSignal piHPSDR patch: {e}")

    if changed:
        print_success("Applied Saturn V3 PureSignal piHPSDR patch")
    else:
        print_info("Saturn V3 PureSignal piHPSDR patch already present")

# Update Git repository
def update_git():
    if args.skip_git:
        print_warning("Skipping repository update")
        return 0
    if args.dry_run:
        print_info("[Dry Run] Simulating Git update")
        return 0
    debug_print("Updating Git repository")
    print_header("Git Update")
    print(f"{Colors.CYAN}⚡ Updating repository...{Colors.END}")
    PIHPSDR_DIR.parent.mkdir(parents=True, exist_ok=True)
    if PIHPSDR_DIR.exists():
        try:
            os.chdir(PIHPSDR_DIR)
            revert_saturn_wdsp2_linux_compatibility()
            revert_saturn_pihpsdr_v3_patch()
            current_commit = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()
            print_info(f"Commit: {current_commit}")
            if subprocess.run(["git", "diff-index", "--quiet", "HEAD", "--"]).returncode != 0:
                print_warning("Stashing changes")
                subprocess.run(["git", "stash", "push", "-m", f"Auto-stash {datetime.now()}"], check=True)
            max_attempts = 3
            git_pull_log = temp_log_path("git_pull_output")
            for attempt in range(1, max_attempts + 1):
                try:
                    with git_pull_log.open("w") as f:
                        process = subprocess.Popen(["git", "pull", "origin", DEFAULT_BRANCH], stdout=f, stderr=f, text=True)
                        return_code = progress_bar(process, "Pulling changes", 50)
                        if return_code == 0:
                            break
                        error_output = git_pull_log.read_text(errors="replace").strip()
                        if attempt < max_attempts:
                            print_warning(f"Git update failed (attempt {attempt}/{max_attempts}): {error_output}. Retrying...")
                            time.sleep(2)
                        else:
                            print_error(f"Git update failed after {max_attempts} attempts: {error_output}")
                    if args.verbose:
                        print_info(f"Git output: {git_pull_log.read_text(errors='replace').strip()}")
                    break
                except subprocess.CalledProcessError as e:
                    if attempt < max_attempts:
                        print_warning(f"Git update failed (attempt {attempt}/{max_attempts}): {e.output.strip()}. Retrying...")
                        time.sleep(2)
                    else:
                        print_error(f"Git update failed after {max_attempts} attempts: {e.output.strip()}")
            new_commit = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()
            if current_commit != new_commit:
                changes = subprocess.check_output(["git", "log", "--oneline", f"{current_commit}..HEAD"], text=True).strip().splitlines()
                print_info(f"New commit: {new_commit}")
                print_info(f"Changes: {len(changes)} commits")
                if args.verbose:
                    print_info(f"Log: {changes}")
            else:
                print_info("Up to date")
            print_success("Repository updated")
        except subprocess.CalledProcessError as e:
            print_error(f"Git update failed: {e.output.strip()}")
    else:
        print(f"{Colors.CYAN}⚡ Cloning repository...{Colors.END}")
        max_attempts = 3
        git_clone_log = temp_log_path("git_clone_output")
        for attempt in range(1, max_attempts + 1):
            try:
                with git_clone_log.open("w") as f:
                    process = subprocess.Popen(["git", "clone", REPO_URL, str(PIHPSDR_DIR)], stdout=f, stderr=f, text=True)
                    return_code = progress_bar(process, "Cloning repository", 50)
                    if return_code == 0:
                        break
                    error_output = git_clone_log.read_text(errors="replace").strip()
                    if attempt < max_attempts:
                        print_warning(f"Git clone failed (attempt {attempt}/{max_attempts}): {error_output}. Retrying...")
                        time.sleep(2)
                    else:
                        print_error(f"Git clone failed after {max_attempts} attempts: {error_output}")
                if args.verbose:
                    print_info(f"Git output: {git_clone_log.read_text(errors='replace').strip()}")
                os.chdir(PIHPSDR_DIR)
                new_commit = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True).strip()
                print_info(f"Commit: {new_commit}")
                print_success("Repository cloned")
                break
            except subprocess.CalledProcessError as e:
                if attempt < max_attempts:
                    print_warning(f"Git clone failed (attempt {attempt}/{max_attempts}): {e.output.strip()}. Retrying...")
                    time.sleep(2)
                else:
                    print_error(f"Git clone failed after {max_attempts} attempts: {e.output.strip()}")
# Build piHPSDR
PIHPSDR_BUILD_PKG_CONFIG_MODULES = (
    "fftw3",
    "fftw3f",
    "libgpiod",
    "alsa",
    "libpulse",
    "libpulse-simple",
    "libpulse-mainloop-glib",
    "miniupnpc",
    "libwebsockets",
    "zlib",
    "opus",
    "sqlite3",
    "libcurl",
    "gtk+-3.0",
    "openssl",
)

def missing_build_dependencies():
    """Return required pkg-config modules not available to the build."""
    if shutil.which("pkg-config") is None:
        return ["pkg-config"]
    return [
        module
        for module in PIHPSDR_BUILD_PKG_CONFIG_MODULES
        if subprocess.run(
            ["pkg-config", "--exists", module],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode != 0
    ]

def can_run_privileged_dependency_installer():
    if os.geteuid() == 0:
        return True
    # init_logging wraps stdout/stderr in Tee, so inspect the original streams.
    if sys.__stdin__.isatty() and sys.__stdout__.isatty():
        return True
    return subprocess.run(
        ["sudo", "-n", "true"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0

def install_build_dependencies():
    missing = missing_build_dependencies()
    if not missing:
        print_success("Build dependencies already installed")
        return

    print_build_output(f"Missing pkg-config modules: {', '.join(missing)}")
    libinstall_script = PIHPSDR_DIR / "LINUX" / "libinstall.sh"
    if not libinstall_script.exists():
        print_error(f"No libinstall.sh script found at {libinstall_script}")
    if args.dry_run:
        print_info(f"[Dry Run] Would run {libinstall_script}")
        return
    if not can_run_privileged_dependency_installer():
        print_error("Missing dependencies require an interactive terminal or passwordless sudo")

    output_filter = DependencyOutputFilter() if args.verbose else None
    if args.verbose:
        process = subprocess.Popen(
            ["bash", str(libinstall_script)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            universal_newlines=True,
            env=dependency_env(),
        )
        return_code = stream_process_output(process, output_filter)
        if output_filter.suppressed:
            print_info(
                f"Compacted dependency output: suppressed {output_filter.suppressed} routine apt/debconf lines"
            )
        if output_filter.saw_autoremove_notice:
            print_warning("APT reports auto-removable packages; not running autoremove automatically")
    else:
        libinstall_log = temp_log_path("libinstall_output")
        with libinstall_log.open("w") as log_file:
            process = subprocess.Popen(
                ["bash", str(libinstall_script)],
                stdout=log_file,
                stderr=log_file,
                text=True,
                env=dependency_env(),
            )
            return_code = progress_bar(process, "Installing dependencies", 50)
        if return_code != 0:
            print_error(f"Dependency installation failed; see {libinstall_log}")

    remaining = missing_build_dependencies()
    if return_code != 0 or remaining:
        if remaining:
            print_build_output(f"Still missing: {', '.join(remaining)}")
        print_error("Dependency installation did not complete successfully")
    print_success("Dependencies installed")

def build_pihpsdr():
    debug_print("Building piHPSDR")
    print_header("piHPSDR Build")
    try:
        os.chdir(PIHPSDR_DIR)
        print(f"{Colors.CYAN}⚡ Cleaning build...{Colors.END}")
        if not args.dry_run:
            if args.verbose:
                process = subprocess.Popen(["make", "clean"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, universal_newlines=True)
                while process.poll() is None:
                    line = process.stdout.readline().strip()
                    if line:
                        print_build_output(line)
                return_code = process.wait()
                if return_code != 0:
                    print_error(f"make clean failed: Check log for details")
            else:
                make_clean_log = temp_log_path("make_clean_output")
                with make_clean_log.open("w") as f:
                    process = subprocess.Popen(["make", "clean"], stdout=f, stderr=f, text=True)
                    return_code = progress_bar(process, "Cleaning build", 50)
                    if return_code != 0:
                        error_output = make_clean_log.read_text(errors="replace").strip()
                        print_error(f"make clean failed: {error_output}")
        else:
            print_info("[Dry Run] Simulating make clean")
        print_success("Build cleaned")

        print(f"{Colors.CYAN}⚡ Installing dependencies...{Colors.END}")
        install_build_dependencies()

        print(f"{Colors.CYAN}⚡ Building piHPSDR...{Colors.END}")
        if args.no_gpio:
            print_info("Building with GPIO disabled")
            if not args.dry_run:
                makefile = PIHPSDR_DIR / "Makefile"
                try:
                    with makefile.open("r") as f:
                        content = f.read()
                    content = content.replace("#CONTROLLER=NO_CONTROLLER", "CONTROLLER=NO_CONTROLLER")
                    with makefile.open("w") as f:
                        f.write(content)
                except Exception as e:
                    print_error(f"Failed to modify Makefile for no-gpio: {str(e)}")
                if args.verbose:
                    process = subprocess.Popen(["make"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, universal_newlines=True)
                    while process.poll() is None:
                        line = process.stdout.readline().strip()
                        if line:
                            print_build_output(line)
                    return_code = process.wait()
                    if return_code != 0:
                        print_error(f"piHPSDR build failed: Check log for details")
                else:
                    make_no_gpio_log = temp_log_path("make_no_gpio_output")
                    with make_no_gpio_log.open("w") as f:
                        process = subprocess.Popen(["make"], stdout=f, stderr=f, text=True)
                        return_code = progress_bar(process, "Building piHPSDR", 50)
                        if return_code != 0:
                            error_output = make_no_gpio_log.read_text(errors="replace").strip()
                            print_error(f"piHPSDR build failed: {error_output}")
            else:
                print_info("[Dry Run] Simulating build with GPIO disabled")
        else:
            if not args.dry_run:
                if args.verbose:
                    process = subprocess.Popen(["make"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, universal_newlines=True)
                    while process.poll() is None:
                        line = process.stdout.readline().strip()
                        if line:
                            print_build_output(line)
                    return_code = process.wait()
                    if return_code != 0:
                        print_error(f"piHPSDR build failed: Check log for details")
                else:
                    make_log = temp_log_path("make_output")
                    with make_log.open("w") as f:
                        process = subprocess.Popen(["make"], stdout=f, stderr=f, text=True)
                        return_code = progress_bar(process, "Building piHPSDR", 50)
                        if return_code != 0:
                            error_output = make_log.read_text(errors="replace").strip()
                            print_error(f"piHPSDR build failed: {error_output}")
            else:
                print_info("[Dry Run] Simulating piHPSDR build")
        print_success("piHPSDR built")
    except Exception as e:
        print_error(f"Build failed: {str(e)}")

# System stats
def get_system_stats():
    debug_print("Getting system stats")
    try:
        cpu = psutil.cpu_percent(interval=1)
        mem = psutil.virtual_memory()
        disk = psutil.disk_usage(str(Path.home()))
        cols, _ = get_term_size()
        stats_text = truncate_text(f"CPU: {cpu:.0f}% | Mem: {mem.used / 1024**2:.0f}/{mem.total / 1024**2:.0f}MB | Disk: {disk.used / 1024**3:.1f}/{disk.total / 1024**3:.1f}G", cols-7)
        print_info(stats_text)
    except Exception as e:
        print_warning(f"Failed to retrieve system stats: {str(e)}")

# Print summary report
def print_summary_report(start_time, backup_created):
    debug_print("Printing summary report")
    print_header("Summary")
    cols, _ = get_term_size()
    completed_text = truncate_text(f"Completed: {datetime.now()}", cols-7)
    duration_text = truncate_text(f"Duration: {int(time.time() - start_time)} seconds", cols-7)
    log_filename = f"pihpsdr-update-{TIMESTAMP}.log"
    log_text = truncate_text(f"Log: {LOG_DIR / log_filename}", cols-7)
    backup_text = truncate_text(f"Backup: {BACKUP_DIR}", cols-7)
    print_success(completed_text)
    print_info(duration_text)
    print_info(log_text)
    if backup_created:
        print_success(backup_text)
    else:
        print_warning("No backup created")

# Main execution
def main():
    global args
    args = parse_args()
    guard_repo_tree_python_execution()
    start_time = time.time()
    BACKUP_CREATED = False

    debug_print("Starting main execution")
    init_logging(args.verbose)

    if args.skip_git:
        print_warning("Skipping Git update")
    if args.yes:
        print_success("Backup enabled via -y flag")
    if args.no:
        print_warning("Backup disabled via -n flag")
    if args.no_gpio:
        print_warning("GPIO disabled for Radioberry compatibility")
    if args.dry_run:
        print_warning("Dry run enabled")
    if args.verbose:
        print_info("Verbose output enabled for all commands, including detailed compile output")
    if args.debug:
        print_info("Debug output enabled")

    check_requirements()
    check_connectivity()
    if PIHPSDR_DIR.exists():
        BACKUP_CREATED = create_backup()
    update_git()
    apply_saturn_pihpsdr_v3_patch()
    apply_saturn_wdsp2_linux_compatibility()
    build_pihpsdr()
    print_summary_report(start_time, BACKUP_CREATED)
    print_header(f"{SCRIPT_NAME} v{SCRIPT_VERSION} Done")
    get_system_stats()
    print_success("Complete!")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print_error("Script interrupted by user")
    except Exception as e:
        print_error(f"Unexpected error: {str(e)}")
    finally:
        os.chdir(Path.home())
        shutil.rmtree(TMP_DIR, ignore_errors=True)
