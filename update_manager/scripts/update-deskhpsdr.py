#!/usr/bin/env python3
# update-deskhpsdr.py - deskHPSDR Update Script
# Mirrors the piHPSDR updater flow while delegating the actual deskHPSDR build
# to the Saturn repo shell scripts.

import glob
import logging
import os
import re
import shlex
import shutil
import subprocess
import sys
import time
import argparse
from datetime import datetime
from pathlib import Path

import psutil

sys.dont_write_bytecode = True

try:
    from pyfiglet import Figlet
except ImportError:
    Figlet = None


class Colors:
    RED = "\033[31m"
    BLUE = "\033[34m"
    CYAN = "\033[36m"
    GREEN = "\033[32m"
    YELLOW = "\033[33m"
    END = "\033[0m"


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


SCRIPT_NAME = "deskHPSDR Update"
SCRIPT_VERSION = "1.1"
SCRIPT_START_TIME = datetime.now()
TIMESTAMP = SCRIPT_START_TIME.strftime("%Y%m%d-%H%M%S")
DESKHPSDR_DIR = Path.home() / "github" / "deskhpsdr"
LOG_DIR = Path.home() / "saturn-logs"
BACKUP_DIR = Path.home() / f"deskhpsdr-backup-{TIMESTAMP}"
REPO_URL = "https://github.com/dl1bz/deskhpsdr.git"
DEFAULT_BRANCH = "master"
TMP_DIR = Path("/tmp") / f"deskhpsdr-update-{TIMESTAMP}-{os.getpid()}"
PRIVILEGED_DEPS_HELPER = Path("/usr/local/lib/saturn-go/scripts/deskhpsdr-install-deps-on-current-image.sh")


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
    clean_text = "".join(c for c in text if c.isprintable())
    if len(clean_text) > max_len:
        return clean_text[: max_len - 2] + ".."
    return clean_text


def debug_print(msg):
    if args.debug:
        print(f"{Colors.END}[DEBUG] {msg}{Colors.END}")
        logging.debug(msg)


def temp_log_path(name):
    return TMP_DIR / f"{name}.log"


def print_header(title):
    cols, _ = get_term_size()
    title = truncate_text(title, cols - 12)
    print(f"\n{Colors.BLUE}═════ {title} ═════{Colors.END}\n")
    logging.info("Header: %s", title)


def print_success(msg):
    print(f"{Colors.GREEN}✔ {msg}{Colors.END}")
    logging.info("Success: %s", msg)


def print_warning(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols - 7)
    print(f"{Colors.YELLOW}⚠ {msg}{Colors.END}")
    logging.warning(msg)


def print_error(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols - 7)
    print(f"{Colors.RED}✗ {msg}{Colors.END}", file=sys.stderr)
    logging.error(msg)
    sys.exit(1)


def print_info(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols - 7)
    print(f"{Colors.CYAN}ℹ {msg}{Colors.END}")
    logging.info(msg)


def print_build_output(msg):
    cols, _ = get_term_size()
    msg = truncate_text(msg, cols - 7)
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

        return text


def dependency_env():
    env = os.environ.copy()
    env["DEBIAN_FRONTEND"] = "noninteractive"
    env["APT_LISTCHANGES_FRONTEND"] = "none"
    env["NEEDRESTART_MODE"] = "a"
    return env


def progress_bar(pid, msg, total_steps):
    _ = total_steps
    if args.dry_run:
        print_info(f"[Dry Run] Simulating progress for: {msg}")
        return 0
    cols, _ = get_term_size()
    max_width = cols - 20
    msg = truncate_text(msg, max_width)
    print(f"{Colors.CYAN}Progress: {msg}{Colors.END}")
    return pid.wait()


def init_logging(verbose=False):
    debug_print("Initializing logging")
    try:
        LOG_DIR.mkdir(parents=True, exist_ok=True)
        TMP_DIR.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        print_error(f"Failed to initialize log directories: {e}")

    class Tee:
        def __init__(self, *files):
            self.files = files

        def write(self, data):
            for f in self.files:
                try:
                    f.write(data)
                except UnicodeEncodeError:
                    encoding = getattr(f, "encoding", None) or "utf-8"
                    safe_data = data.encode(encoding, errors="replace").decode(
                        encoding, errors="replace"
                    )
                    f.write(safe_data)
                f.flush()

        def flush(self):
            for f in self.files:
                f.flush()

    log_file = LOG_DIR / f"deskhpsdr-update-{TIMESTAMP}.log"
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        handlers=[logging.FileHandler(log_file, encoding="utf-8")],
    )
    try:
        log_handle = open(log_file, "a", encoding="utf-8")
    except Exception as e:
        print_error(f"Failed to open log file {log_file}: {e}")
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
        f = Figlet(font="standard", width=cols - 2, justify="center")
        banner_text = f.renderText("deskHPSDR")
    else:
        banner_text = "deskHPSDR\n"
    banner = f"""
{Colors.RED}{banner_text.rstrip()}{Colors.END}
{Colors.BLUE}{f'Update Manager v{SCRIPT_VERSION}'.center(cols-2)}{Colors.END}\n\n"""
    print(banner)
    print_info(f"Started: {SCRIPT_START_TIME}")
    print_info(f"Log: {log_file}")


def parse_args():
    parser = argparse.ArgumentParser(description="deskHPSDR Update Script")
    parser.add_argument("--skip-git", action="store_true", help="Skip Git repository update")
    parser.add_argument("-y", "--yes", action="store_true", help="Auto-confirm backup creation")
    parser.add_argument("-n", "--no", action="store_true", help="Skip backup creation")
    parser.add_argument("--no-install-deps", action="store_true", help="Skip apt-based dependency installation during the build step")
    parser.add_argument("--no-clean", action="store_true", help="Skip make clean before the build")
    parser.add_argument("--no-desktop-shortcut", action="store_true", help="Skip creating the Desktop shortcut")
    parser.add_argument("--dry-run", action="store_true", help="Simulate actions without executing")
    parser.add_argument("--verbose", action="store_true", help="Enable verbose output for all commands, including detailed build output")
    parser.add_argument("--debug", action="store_true", help="Enable debug output")
    parsed = parser.parse_args()
    if parsed.yes and parsed.no:
        print_error("Cannot use -y and -n together")
    return parsed


def resolve_saturn_repo_root():
    candidates = []
    for candidate in (
        os.environ.get("SATURN_REPO_ROOT"),
        os.environ.get("SATURN_DIR"),
        str(Path.home() / "github" / "Saturn"),
    ):
        if not candidate:
            continue
        root = Path(candidate).expanduser().resolve()
        if root not in candidates:
            candidates.append(root)
    for root in candidates:
        if root.joinpath("scripts").is_dir():
            return root
    tried = ", ".join(str(p) for p in candidates) or "(none)"
    print_error(f"Cannot locate Saturn repo root with scripts/: {tried}")


def resolve_saturn_script(script_name):
    repo_root = resolve_saturn_repo_root()
    script_path = repo_root / "scripts" / script_name
    if not script_path.is_file():
        print_error(f"Required Saturn helper script not found: {script_path}")
    return script_path


def check_privileged_deps_helper():
    if args.no_install_deps:
        return

    if not PRIVILEGED_DEPS_HELPER.is_file():
        print_error(
            f"deskHPSDR dependency helper is not installed: {PRIVILEGED_DEPS_HELPER}. "
            "Reinstall Saturn Go to refresh the privileged helper set."
        )

    cmd = ["sudo", "-n", str(PRIVILEGED_DEPS_HELPER), "--check-only"]
    result = subprocess.run(cmd, text=True, capture_output=True)
    if result.returncode != 0:
        output = "\n".join(part.strip() for part in (result.stdout, result.stderr) if part.strip())
        if output:
            print_warning(output)
        print_error(
            "deskHPSDR dependency helper is not callable through passwordless sudo. "
            "Reinstall Saturn Go to refresh /etc/sudoers.d/saturn-go-maintenance."
        )

    for line in result.stdout.splitlines():
        if line.strip():
            print_info(line.strip())


def check_requirements():
    debug_print("Checking requirements")
    print_header("System Check")
    print(f"{Colors.CYAN}⚡ Verifying requirements...{Colors.END}")
    requirements = ["git", "bash", "sudo", "rsync"]
    for item in requirements:
        print(f"{Colors.CYAN}Scanning: {item}...{Colors.END}")
        time.sleep(0.3)
        if shutil.which(item):
            cols, _ = get_term_size()
            print(f"{Colors.GREEN}✓ {item} - OK{' ' * (cols - len(item) - 8)}{Colors.END}")
        else:
            print_error(f"Missing command: {item}")

    build_script = resolve_saturn_script("deskhpsdr-test-build-on-current-image.sh")
    install_script = resolve_saturn_script("deskhpsdr-install-on-current-image.sh")
    print_success(f"Build helper: {build_script}")
    print_success(f"Install helper: {install_script}")
    check_privileged_deps_helper()

    try:
        free_space = psutil.disk_usage(str(Path.home())).free / 1024**3
        if free_space < 1:
            print_warning(f"Low disk space: {free_space:.2f}GB")
        else:
            print_success(f"Disk: {free_space:.2f}GB free")
        print_success("Requirements met")
    except Exception as e:
        print_error(f"Failed to check disk space: {e}")


def check_connectivity():
    debug_print("Checking connectivity")
    if args.skip_git:
        print_warning("Skipping network check")
        return 0
    if not shutil.which("ping"):
        print_warning("ping not found; skipping network check")
        return 0
    print_header("Network Check")
    print(f"{Colors.CYAN}⚡ Checking connectivity...{Colors.END}")
    max_attempts = 3
    for attempt in range(1, max_attempts + 1):
        try:
            result = subprocess.run(
                ["ping", "-c", "1", "-W", "2", "github.com"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            rtt_match = re.search(r"time=([\d.]+)\s*ms", result.stdout)
            if args.verbose and rtt_match:
                print_info(f"RTT: {float(rtt_match.group(1)):.3f} ms")
            print_success("Network verified")
            return 0
        except subprocess.CalledProcessError as e:
            if attempt < max_attempts:
                print_warning(
                    f"Cannot reach GitHub (attempt {attempt}/{max_attempts}): "
                    f"{e.output.strip()}. Retrying..."
                )
                time.sleep(2)
            else:
                print_warning(
                    f"Cannot reach GitHub after {max_attempts} attempts: {e.output.strip()}"
                )
                return 1


def create_backup():
    debug_print("Creating backup")
    print_header("Backup")
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

    if not do_backup:
        return False

    print(f"{Colors.CYAN}⚡ Creating backup...{Colors.END}")
    backup_pattern = str(Path.home() / "deskhpsdr-backup-*")
    backup_dirs = sorted(glob.glob(backup_pattern), key=os.path.getmtime, reverse=True)
    print_info(f"Found {len(backup_dirs)} existing backups")
    if len(backup_dirs) > 2:
        for old_backup in backup_dirs[2:]:
            try:
                if not args.dry_run:
                    shutil.rmtree(old_backup)
                print_info(f"Deleted old backup: {old_backup}")
            except Exception as e:
                print_warning(f"Failed to delete backup {old_backup}: {e}")

    print_info(f"Location: {BACKUP_DIR}")
    try:
        if not args.dry_run:
            BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        print_error(f"Cannot create backup dir: {e}")

    rsync_log = temp_log_path("rsync_output")
    try:
        with rsync_log.open("w", encoding="utf-8") as f:
            process = subprocess.Popen(
                ["rsync", "-a", f"{DESKHPSDR_DIR}/", str(BACKUP_DIR)],
                stdout=f,
                stderr=f,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            return_code = progress_bar(process, "Copying files", 50)
            if return_code != 0:
                error_output = rsync_log.read_text(errors="replace").strip()
                print_error(f"Backup failed: {error_output}")
            if args.verbose:
                print_info(f"Rsync output: {rsync_log.read_text(errors='replace').strip()}")
    except Exception as e:
        print_error(f"Failed to access {rsync_log}: {e}")

    if args.dry_run:
        print_info("[Dry Run] Backup created")
        return True

    backup_size = sum(f.stat().st_size for f in BACKUP_DIR.rglob("*") if f.is_file()) / 1024**2
    print_info(f"Size: {backup_size:.1f}MB")
    print_success("Backup created")
    return True


def git_output_or_die(log_path: Path, context: str):
    try:
        text = log_path.read_text(errors="replace").strip()
    except Exception:
        text = ""
    if text:
        print_error(f"{context}: {text}")
    print_error(f"{context}: Check log for details")


def update_git():
    if args.skip_git:
        print_warning("Skipping repository update")
        if not DESKHPSDR_DIR.exists():
            print_error(f"deskHPSDR checkout not found: {DESKHPSDR_DIR}")
        return 0

    if args.dry_run:
        print_info("[Dry Run] Simulating Git update")
        return 0

    debug_print("Updating Git repository")
    print_header("Git Update")
    print(f"{Colors.CYAN}⚡ Updating repository...{Colors.END}")
    DESKHPSDR_DIR.parent.mkdir(parents=True, exist_ok=True)

    if DESKHPSDR_DIR.exists() and not DESKHPSDR_DIR.joinpath(".git").is_dir():
        print_error(f"Target exists but is not a git checkout: {DESKHPSDR_DIR}")

    if DESKHPSDR_DIR.exists():
        try:
            os.chdir(DESKHPSDR_DIR)
            current_commit = subprocess.check_output(
                ["git", "rev-parse", "--short", "HEAD"], text=True
            ).strip()
            current_branch = subprocess.check_output(
                ["git", "branch", "--show-current"], text=True
            ).strip() or DEFAULT_BRANCH
            print_info(f"Commit: {current_commit}")
            print_info(f"Branch: {current_branch}")

            try:
                remote_url = subprocess.check_output(
                    ["git", "remote", "get-url", "origin"], text=True
                ).strip()
                if remote_url != REPO_URL:
                    print_warning(f"Origin URL differs from expected repo: {remote_url}")
            except subprocess.CalledProcessError:
                print_warning("Unable to read origin URL")

            if subprocess.run(
                ["git", "diff-index", "--quiet", "HEAD", "--"],
                check=False,
            ).returncode != 0:
                print_warning("Stashing changes")
                subprocess.run(
                    ["git", "stash", "push", "-m", f"Auto-stash {datetime.now()}"],
                    check=True,
                )

            git_pull_log = temp_log_path("git_pull_output")
            max_attempts = 3
            for attempt in range(1, max_attempts + 1):
                with git_pull_log.open("w", encoding="utf-8") as f:
                    process = subprocess.Popen(
                        ["git", "pull", "--ff-only", "origin", current_branch],
                        stdout=f,
                        stderr=f,
                        text=True,
                        encoding="utf-8",
                        errors="replace",
                    )
                    return_code = progress_bar(process, "Pulling changes", 50)
                if return_code == 0:
                    break
                if attempt < max_attempts:
                    error_output = git_pull_log.read_text(errors="replace").strip()
                    print_warning(
                        f"Git update failed (attempt {attempt}/{max_attempts}): "
                        f"{error_output}. Retrying..."
                    )
                    time.sleep(2)
                else:
                    git_output_or_die(git_pull_log, f"Git update failed after {max_attempts} attempts")

            if args.verbose:
                print_info(f"Git output: {git_pull_log.read_text(errors='replace').strip()}")

            new_commit = subprocess.check_output(
                ["git", "rev-parse", "--short", "HEAD"], text=True
            ).strip()
            if current_commit != new_commit:
                changes = subprocess.check_output(
                    ["git", "log", "--oneline", f"{current_commit}..HEAD"],
                    text=True,
                ).strip().splitlines()
                print_info(f"New commit: {new_commit}")
                print_info(f"Changes: {len(changes)} commits")
                if args.verbose:
                    print_info(f"Log: {changes}")
            else:
                print_info("Up to date")
            print_success("Repository updated")
        except subprocess.CalledProcessError as e:
            print_error(f"Git update failed: {e}")
    else:
        print(f"{Colors.CYAN}⚡ Cloning repository...{Colors.END}")
        git_clone_log = temp_log_path("git_clone_output")
        max_attempts = 3
        for attempt in range(1, max_attempts + 1):
            with git_clone_log.open("w", encoding="utf-8") as f:
                process = subprocess.Popen(
                    ["git", "clone", REPO_URL, str(DESKHPSDR_DIR)],
                    stdout=f,
                    stderr=f,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                )
                return_code = progress_bar(process, "Cloning repository", 50)
            if return_code == 0:
                break
            if attempt < max_attempts:
                error_output = git_clone_log.read_text(errors="replace").strip()
                print_warning(
                    f"Git clone failed (attempt {attempt}/{max_attempts}): "
                    f"{error_output}. Retrying..."
                )
                time.sleep(2)
            else:
                git_output_or_die(git_clone_log, f"Git clone failed after {max_attempts} attempts")

        if args.verbose:
            print_info(f"Git output: {git_clone_log.read_text(errors='replace').strip()}")
        os.chdir(DESKHPSDR_DIR)
        new_commit = subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"], text=True
        ).strip()
        print_info(f"Commit: {new_commit}")
        print_success("Repository cloned")


def run_command(cmd, label, log_name):
    log_path = temp_log_path(log_name)
    if args.dry_run:
        print_info(f"[Dry Run] Simulating: {shlex.join(cmd)}")
        return

    if args.verbose:
        output_filter = (
            DependencyOutputFilter()
            if log_name == "deskhpsdr_build_output"
            else None
        )
        with log_path.open("w", encoding="utf-8") as f:
            process = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=dependency_env(),
            )
            if process.stdout is None:
                print_error(f"{label} failed: no output stream")
            for raw_line in process.stdout:
                f.write(raw_line)
                line = raw_line.rstrip("\n")
                if output_filter is not None:
                    line = output_filter.filter(line)
                elif not line.strip():
                    line = None
                if line:
                    print_build_output(line)
            return_code = process.wait()
        if output_filter is not None:
            if output_filter.suppressed:
                print_info(
                    f"Compacted dependency output: suppressed {output_filter.suppressed} routine apt/debconf lines"
                )
            if output_filter.saw_autoremove_notice:
                print_warning("APT reports auto-removable packages; not running autoremove automatically")
    else:
        with log_path.open("w", encoding="utf-8") as f:
            process = subprocess.Popen(
                cmd,
                stdout=f,
                stderr=f,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=dependency_env(),
            )
            return_code = progress_bar(process, label, 50)

    if return_code != 0:
        error_output = log_path.read_text(errors="replace").strip()
        if error_output:
            print_error(f"{label} failed: {error_output}")
        print_error(f"{label} failed: Check log for details")

    if args.verbose:
        print_info(f"Log saved: {log_path}")


def build_deskhpsdr():
    debug_print("Building deskHPSDR")
    print_header("deskHPSDR Build")
    if not DESKHPSDR_DIR.exists():
        print_error(f"deskHPSDR checkout not found: {DESKHPSDR_DIR}")

    build_script = resolve_saturn_script("deskhpsdr-test-build-on-current-image.sh")
    install_script = resolve_saturn_script("deskhpsdr-install-on-current-image.sh")
    print_info(f"Using build helper: {build_script}")
    print_info(f"Installer defaults reference: {install_script}")

    cmd = ["bash", str(build_script), "--repo", str(DESKHPSDR_DIR)]
    if not args.no_install_deps:
        cmd.append("--install-deps")
    if args.no_clean:
        cmd.append("--no-clean")
    if args.no_desktop_shortcut:
        cmd.append("--no-desktop-shortcut")

    print(f"{Colors.CYAN}⚡ Building deskHPSDR...{Colors.END}")
    run_command(cmd, "Building deskHPSDR", "deskhpsdr_build_output")
    print_success("deskHPSDR built")


def get_system_stats():
    debug_print("Getting system stats")
    try:
        cpu = psutil.cpu_percent(interval=1)
        mem = psutil.virtual_memory()
        disk = psutil.disk_usage(str(Path.home()))
        cols, _ = get_term_size()
        stats_text = truncate_text(
            (
                f"CPU: {cpu:.0f}% | Mem: {mem.used / 1024**2:.0f}/"
                f"{mem.total / 1024**2:.0f}MB | Disk: "
                f"{disk.used / 1024**3:.1f}/{disk.total / 1024**3:.1f}G"
            ),
            cols - 7,
        )
        print_info(stats_text)
    except Exception as e:
        print_warning(f"Failed to retrieve system stats: {e}")


def print_summary_report(start_time, backup_created):
    debug_print("Printing summary report")
    print_header("Summary")
    cols, _ = get_term_size()
    completed_text = truncate_text(f"Completed: {datetime.now()}", cols - 7)
    duration_text = truncate_text(f"Duration: {int(time.time() - start_time)} seconds", cols - 7)
    log_filename = f"deskhpsdr-update-{TIMESTAMP}.log"
    log_text = truncate_text(f"Log: {LOG_DIR / log_filename}", cols - 7)
    backup_text = truncate_text(f"Backup: {BACKUP_DIR}", cols - 7)
    print_success(completed_text)
    print_info(duration_text)
    print_info(log_text)
    if backup_created:
        print_success(backup_text)
    else:
        print_warning("No backup created")


def main():
    global args
    args = parse_args()
    guard_repo_tree_python_execution()
    start_time = time.time()
    backup_created = False

    debug_print("Starting main execution")
    init_logging(args.verbose)

    if args.skip_git:
        print_warning("Skipping Git update")
    if args.yes:
        print_success("Backup enabled via -y flag")
    if args.no:
        print_warning("Backup disabled via -n flag")
    if args.no_install_deps:
        print_warning("Dependency installation disabled")
    if args.no_clean:
        print_warning("Build clean step disabled")
    if args.no_desktop_shortcut:
        print_warning("Desktop shortcut creation disabled")
    if args.dry_run:
        print_warning("Dry run enabled")
    if args.verbose:
        print_info("Verbose output enabled for all commands, including detailed build output")
    if args.debug:
        print_info("Debug output enabled")

    check_requirements()
    check_connectivity()
    if DESKHPSDR_DIR.exists():
        backup_created = create_backup()
    update_git()
    build_deskhpsdr()
    print_summary_report(start_time, backup_created)
    print_header(f"{SCRIPT_NAME} v{SCRIPT_VERSION} Done")
    get_system_stats()
    print_success("Complete!")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print_error("Script interrupted by user")
    except Exception as e:
        print_error(f"Unexpected error: {e}")
    finally:
        os.chdir(Path.home())
        shutil.rmtree(TMP_DIR, ignore_errors=True)
