#!/bin/bash
# update-G2.sh - Enhanced Version with Menu System
# Pull repository and build p2app with comprehensive error handling and safety features
# Original script: Laurence Barker G8NJJ
# Substantially rewritten by: KD4YAL
# Enhanced version with logging, backup, rollback capabilities, and menu system

# Exit on any error and handle undefined variables
set -euo pipefail

#############################################################################
# CONFIGURATION AND SETUP
#############################################################################

# Version
SCRIPT_VERSION="2.1"

# Load configuration if exists
CONFIG_FILE="$HOME/.saturn-update.conf"
if [ -f "$CONFIG_FILE" ]; then
    source "$CONFIG_FILE"
fi

# Default configuration (can be overridden by config file)
SATURN_DIR="${SATURN_DIR:-$HOME/github/Saturn}"
PIHPSDR_DIR="${PIHPSDR_DIR:-$HOME/github/pihpsdr}"
SATURN_REPO_URL="${SATURN_REPO_URL:-https://github.com/laurencebarker/Saturn.git}"
SATURN_BRANCH="${SATURN_BRANCH:-main}"
CREATE_BACKUP="${CREATE_BACKUP:-true}"
SKIP_CONNECTIVITY_CHECK="${SKIP_CONNECTIVITY_CHECK:-false}"
VERBOSE="${VERBOSE:-false}"

# Setup logging
LOG_DIR="$HOME/saturn-logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/saturn-update-$(date +%Y%m%d-%H%M%S).log"
BACKUP_INFO_FILE="$HOME/.saturn-last-backup"

# Redirect output to both console and log file
exec 1> >(tee -a "$LOG_FILE")
exec 2> >(tee -a "$LOG_FILE" >&2)

# Command line options
SKIP_UPDATE=false
FORCE_UPDATE=false
SKIP_BACKUP=false

# Check for whiptail
if ! command -v whiptail &> /dev/null; then
    echo "✗ Error: whiptail is required but not installed"
    echo "Please install whiptail (usually part of newt package)"
    exit 1
fi

#############################################################################
# UTILITY FUNCTIONS
#############################################################################

# Function to print section headers
print_header() {
    clear
    echo ""
    echo "##############################################################"
    echo ""
    echo "$1"
    echo ""
    echo "##############################################################"
}

# Enhanced status checking with detailed error reporting
check_status() {
    local exit_code=$?
    local operation="$1"

    if [ $exit_code -eq 0 ]; then
        echo "✓ $operation completed successfully"
        return 0
    else
        echo "✗ Error: $operation failed (exit code: $exit_code)"
        echo "Check log file: $LOG_FILE"
        return $exit_code
    fi
}

# Show progress indicator
show_progress() {
    local current=$1
    local total=$2
    local desc=$3
    local percent=$((current * 100 / total))
    printf "\r[%d/%d] %s... %d%%" "$current" "$total" "$desc" "$percent"
    if [ "$current" -eq "$total" ]; then
        echo ""
    fi
}

# Check system requirements
check_system_requirements() {
    print_header "Checking System Requirements"

    # Check disk space (need at least 1GB free)
    local free_space=$(df "$HOME" | awk 'NR==2 {print $4}')
    if [ "$free_space" -lt 1048576 ]; then
        echo "⚠ Warning: Low disk space (less than 1GB free)"
        echo "Available: $((free_space / 1024))MB"
        whiptail --title "Warning" --msgbox "Low disk space (less than 1GB free)\nAvailable: $((free_space / 1024))MB" 10 60
    else
        echo "✓ Sufficient disk space available"
    fi

    # Check required commands
    local required_commands=("git" "make" "gcc" "sudo" "whiptail")
    local missing_commands=()

    for cmd in "${required_commands[@]}"; do
        if ! command -v "$cmd" &> /dev/null; then
            missing_commands+=("$cmd")
        fi
    done

    if [ ${#missing_commands[@]} -gt 0 ]; then
        echo "✗ Error: Missing required commands: ${missing_commands[*]}"
        echo "Please install missing packages and try again"
        whiptail --title "Error" --msgbox "Missing required commands: ${missing_commands[*]}\nPlease install missing packages and try again" 10 60
        exit 1
    else
        echo "✓ All required commands available"
    fi
}

# Check internet connectivity
check_connectivity() {
    if [ "$SKIP_CONNECTIVITY_CHECK" = "true" ]; then
        return 0
    fi

    echo "Checking internet connectivity..."
    if timeout 10 ping -c 1 github.com &> /dev/null; then
        echo "✓ Internet connectivity confirmed"
        return 0
    else
        echo "⚠ Warning: Cannot reach GitHub"
        whiptail --title "Warning" --msgbox "Cannot reach GitHub" 8 60
        return 1
    fi
}

# Git safety checks and repository validation
check_git_status() {
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        echo "✗ Error: $SATURN_DIR is not a git repository"
        whiptail --title "Error" --msgbox "$SATURN_DIR is not a git repository" 8 60
        exit 1
    fi

    # Validate remote repository
    local current_url=$(git remote get-url origin 2>/dev/null || echo "")
    if [ -z "$current_url" ]; then
        echo "⚠ Warning: No 'origin' remote found"
        echo "Setting up origin remote: $SATURN_REPO_URL"
        git remote add origin "$SATURN_REPO_URL"
        whiptail --title "Warning" --msgbox "No 'origin' remote found\nSetting up origin remote: $SATURN_REPO_URL" 10 60
    else
        echo "✓ Current repository: $current_url"
        if [[ "$current_url" != *"Saturn"* ]] && [[ "$current_url" != "$SATURN_REPO_URL" ]]; then
            echo "⚠ Warning: Repository URL doesn't appear to be the Saturn project"
            echo "Expected: $SATURN_REPO_URL"
            echo "Current: $current_url"

            if [ "$FORCE_UPDATE" = "false" ]; then
                if ! whiptail --title "Warning" --yesno "Repository URL doesn't appear to be the Saturn project\nExpected: $SATURN_REPO_URL\nCurrent: $current_url\nContinue with current repository?" 12 60; then
                    echo "Update cancelled by user"
                    whiptail --title "Info" --msgbox "Update cancelled by user" 8 60
                    exit 0
                fi
            fi
        fi
    fi

    # Check current branch
    local current_branch=$(git branch --show-current 2>/dev/null || echo "unknown")
    echo "✓ Current branch: $current_branch"

    # Check for uncommitted changes
    if ! git diff-index --quiet HEAD --; then
        echo "⚠ Warning: Uncommitted changes detected"
        if [ "$FORCE_UPDATE" = "false" ]; then
            if whiptail --title "Warning" --yesno "Uncommitted changes detected\nStash changes and continue?" 10 60; then
                echo "Stashing changes..."
                git stash push -m "Auto-stash before update $(date)"
                echo "✓ Changes stashed"
                whiptail --title "Success" --msgbox "Changes stashed" 8 60
            else
                echo "Update cancelled by user"
                whiptail --title "Info" --msgbox "Update cancelled by user" 8 60
                exit 0
            fi
        fi
    fi

    # Check if we're on the expected branch
    if [ "$current_branch" != "$SATURN_BRANCH" ] && [ "$SATURN_BRANCH" != "current" ]; then
        echo "⚠ Warning: Currently on branch '$current_branch', expected '$SATURN_BRANCH'"
        if [ "$FORCE_UPDATE" = "false" ]; then
            if whiptail --title "Warning" --yesno "Currently on branch '$current_branch', expected '$SATURN_BRANCH'\nSwitch to branch '$SATURN_BRANCH'?" 10 60; then
                git checkout "$SATURN_BRANCH" || git checkout -b "$SATURN_BRANCH" "origin/$SATURN_BRANCH"
            fi
        fi
    fi
}

# Prompt user for backup preference
prompt_for_backup() {
    if [ "$SKIP_BACKUP" = "true" ] || [ "$FORCE_UPDATE" = "true" ]; then
        return 0
    fi

    echo ""
    echo "🔄 Saturn Update Process Starting"
    echo ""
    echo "It's recommended to create a backup before updating in case you need to rollback."
    echo "This will copy your current Saturn directory to a timestamped backup folder."
    echo ""

    # Check if previous backups exist
    local backup_pattern="$HOME/saturn-backup-*"
    local backup_count=$(ls -1d $backup_pattern 2>/dev/null | wc -l || echo "0")
    local latest_backup=$(ls -1dt $backup_pattern 2>/dev/null | head -1 | xargs basename 2>/dev/null || echo "None")

    if [ "$backup_count" -gt 0 ]; then
        echo "📁 Found $backup_count existing backup(s)"
        echo "💾 Latest backup: $latest_backup"
        echo ""
    fi

    # Estimate backup size
    local saturn_size=$(du -sh "$SATURN_DIR" 2>/dev/null | cut -f1 || echo "Unknown")
    echo "📊 Current Saturn directory size: $saturn_size"

    # Create Whiptail message
    local message="Saturn Update Process Starting\n\n"
    message+="It's recommended to create a backup before updating in case you need to rollback.\n"
    message+="This will copy your current Saturn directory to a timestamped backup folder.\n\n"
    message+="Found $backup_count existing backup(s)\n"
    message+="Latest backup: $latest_backup\n\n"
    message+="Current Saturn directory size: $saturn_size\n\n"
    message+="Would you like to create a backup before updating?"

    if ! whiptail --title "Saturn Update" --yesno "$message" 16 60; then
        echo "⚠ Proceeding without backup"
        SKIP_BACKUP=true
        whiptail --title "Info" --msgbox "Proceeding without backup" 8 60
        return 0
    else
        echo "✓ Backup will be created"
        return 0
    fi
}

# Create backup
create_backup() {
    if [ "$SKIP_BACKUP" = "true" ]; then
        echo "Skipping backup creation (user choice or --skip-backup flag)"
        whiptail --title "Info" --msgbox "Skipping backup creation (user choice or --skip-backup flag)" 8 60
        return 0
    fi

    print_header "Creating Backup"
    local backup_dir="$HOME/saturn-backup-$(date +%Y%m%d-%H%M%S)"

    echo "Creating backup at: $backup_dir"
    echo "This may take a few moments depending on the size of your Saturn directory..."

    # Show progress in Whiptail
    {
        echo 0
        if command -v rsync &> /dev/null; then
            rsync -av --progress "$SATURN_DIR/" "$backup_dir/" 2>/dev/null || cp -r "$SATURN_DIR" "$backup_dir"
        else
            cp -r "$SATURN_DIR" "$backup_dir"
        fi
        echo 100
    } | whiptail --title "Backup Progress" --gauge "Creating backup at: $backup_dir" 6 60 0

    # Save backup info for potential rollback
    echo "BACKUP_DIR=$backup_dir" > "$BACKUP_INFO_FILE"
    echo "BACKUP_DATE=$(date)" >> "$BACKUP_INFO_FILE"
    echo "BACKUP_SIZE=$(du -sh "$backup_dir" 2>/dev/null | cut -f1 || echo "Unknown")" >> "$BACKUP_INFO_FILE"

    echo "✓ Backup created successfully"
    echo "📁 Location: $backup_dir"
    echo "📊 Size: $(du -sh "$backup_dir" 2>/dev/null | cut -f1 || echo "Unknown")"
    whiptail --title "Success" --msgbox "Backup created successfully\nLocation: $backup_dir\nSize: $(du -sh "$backup_dir" 2>/dev/null | cut -f1 || echo "Unknown")" 10 60
}

# Rollback to previous backup
rollback_changes() {
    if [ ! -f "$BACKUP_INFO_FILE" ]; then
        echo "✗ No backup information found"
        whiptail --title "Error" --msgbox "No backup information found" 8 60
        return 1
    fi

    source "$BACKUP_INFO_FILE"

    if [ ! -d "$BACKUP_DIR" ]; then
        echo "✗ Backup directory not found: $BACKUP_DIR"
        whiptail --title "Error" --msgbox "Backup directory not found: $BACKUP_DIR" 8 60
        return 1
    fi

    print_header "Rolling Back Changes"
    echo "Restoring from backup: $BACKUP_DIR"
    echo "Backup date: $BACKUP_DATE"

    rm -rf "$SATURN_DIR"
    cp -r "$BACKUP_DIR" "$SATURN_DIR"

    echo "✓ Rollback completed successfully"
    whiptail --title "Success" --msgbox "Rollback completed successfully" 8 60
}

# Cleanup old backups (keep last 5)
cleanup_old_backups() {
    local backup_pattern="$HOME/saturn-backup-*"
    local backup_count=$(ls -1d $backup_pattern 2>/dev/null | wc -l)

    if [ "$backup_count" -gt 5 ]; then
        echo "Cleaning up old backups (keeping last 5)..."
        ls -1dt $backup_pattern | tail -n +6 | xargs rm -rf
        echo "✓ Old backups cleaned up"
        whiptail --title "Success" --msgbox "Old backups cleaned up (kept last 5)" 8 60
    fi
}

# Show usage information
show_usage() {
    local usage="Saturn Update Script v$SCRIPT_VERSION\n\n"
    usage+="Usage: $0 [OPTIONS]\n\n"
    usage+="Options:\n"
    usage+="  --skip-git          Skip git pull operation\n"
    usage+="  --skip-backup       Skip backup creation (not recommended)\n"
    usage+="  --force            Force update without prompts (includes backup skip)\n"
    usage+="  --verbose          Enable verbose output\n"
    usage+="  --rollback         Rollback to previous backup\n"
    usage+="  --repo-url URL     Override repository URL\n"
    usage+="  --branch BRANCH    Override target branch (default: main)\n"
    usage+="  --help             Show this help message\n\n"
    usage+="Environment variables:\n"
    usage+="  SATURN_REPO_URL    Repository URL (default: https://github.com/laurencebarker/Saturn.git)\n"
    usage+="  SATURN_BRANCH      Target branch (default: main)\n\n"
    usage+="Configuration file: $CONFIG_FILE\n"
    usage+="Log file: $LOG_FILE"

    whiptail --title "Help" --msgbox "$usage" 20 70
}

# Function to get current Git revision
get_git_revision() {
    (cd "$SATURN_DIR" 2>/dev/null && git rev-parse --short HEAD) || echo "Unknown"
}

# Function to find latest FPGA BIT file
get_fpga_bit_file() {
    (cd "$SATURN_DIR" 2>/dev/null && ./scripts/find-bin.sh) || echo "Not detected"
}

#############################################################################
# CORE TASK FUNCTIONS
#############################################################################

update_libraries() {
    print_header "Updating System Libraries"
    if [ -f "$SATURN_DIR/scripts/install-libraries.sh" ]; then
        "$SATURN_DIR/scripts/install-libraries.sh"
        check_status "Library installation"
    else
        echo "Error: Library script not found"
        return 1
    fi
}

update_repository() {
    print_header "Updating Git Repository"
    cd "$SATURN_DIR" || return 1
    local current_commit
    current_commit=$(git rev-parse HEAD)
    git pull
    check_status "Git repository update"
    [ "$current_commit" != "$(git rev-parse HEAD)" ] && echo "New updates were fetched"
}

build_p2app() {
    print_header "Building p2app"
    if [ -f "$SATURN_DIR/scripts/update-p2app.sh" ]; then
        "$SATURN_DIR/scripts/update-p2app.sh"
        check_status "p2app build"
    else
        echo "Error: p2app build script not found"
        return 1
    fi
}

build_desktop_apps() {
    print_header "Building Desktop Applications"
    if [ -f "$SATURN_DIR/scripts/update-desktop-apps.sh" ]; then
        "$SATURN_DIR/scripts/update-desktop-apps.sh"
        check_status "Desktop apps build"
    else
        echo "Error: Desktop apps script not found"
        return 1
    fi
}

install_udev_rules() {
    print_header "Installing Udev Rules"
    if [ -f "$SATURN_DIR/rules/install-rules.sh" ]; then
        sudo "$SATURN_DIR/rules/install-rules.sh"
        check_status "Udev rules installation"
    else
        echo "Error: Udev rules script not found"
        return 1
    fi
}

copy_desktop_icons() {
    print_header "Copying Desktop Icons"
    if [ -d "$SATURN_DIR/desktop" ] && [ -d "$HOME/Desktop" ]; then
        cp -v "$SATURN_DIR"/desktop/* "$HOME/Desktop/"
        check_status "Desktop icons copy"
    else
        echo "Error: Desktop directory missing"
        return 1
    fi
}

check_fpga_bit_file() {
    print_header "Verifying FPGA BIT File"
    get_fpga_bit_file
    check_status "FPGA file check"
}

build_pihpsdr() {
    print_header "Building pihpsdr"
    if [ -d "$PIHPSDR_DIR" ]; then
        cd "$PIHPSDR_DIR" || return 1
        make clean
        check_status "Clean build" || return 1
        git pull
        check_status "Repository update" || return 1
        make
        check_status "pihpsdr compilation"
    else
        echo "Error: pihpsdr directory missing"
        return 1
    fi
}

perform_all_tasks() {
    local status=0
    update_libraries || status=$?
    update_repository || status=$?
    build_p2app || status=$?
    build_desktop_apps || status=$?
    install_udev_rules || status=$?
    copy_desktop_icons || status=$?
    check_fpga_bit_file || status=$?
    build_pihpsdr || status=$?
    return $status
}

#############################################################################
# run_function Wrapper for GUI Feedback
#############################################################################

run_function() {
    local func=$1
    # Capture the function's output
    local output
    output=$($func 2>&1)
    local status=$?

    # Write output to a temporary file
    local tmpfile
    tmpfile=$(mktemp /tmp/operation_results.XXXXXX)
    {
        echo "Results of '$func':"
        echo "----------------------------------------"
        echo "$output"
        echo "----------------------------------------"
        if [ $status -eq 0 ]; then
            echo "SUCCESS"
        else
            echo "FAILED (code: $status)"
        fi
    } > "$tmpfile"

    # Display the results using whiptail --textbox
    whiptail --title "Operation Results" --scrolltext --textbox "$tmpfile" 20 60
    rm "$tmpfile"
}

#############################################################################
# GUI Menu System
#############################################################################

main_menu() {
    while true; do
        local GIT_REVISION FPGA_BIT_FILE
        GIT_REVISION=$(get_git_revision)
        FPGA_BIT_FILE=$(get_fpga_bit_file)

        local choice
        choice=$(whiptail --title "Saturn Update System" \
            --menu "Current Status:\n\nGit Revision: $GIT_REVISION\nFPGA Firmware: $FPGA_BIT_FILE" \
            22 60 12 \
            "1" "Update System Libraries" \
            "2" "Update Software Repository" \
            "3" "Build p2app" \
            "4" "Build Desktop Apps" \
            "5" "Install Device Rules" \
            "6" "Update Desktop Icons" \
            "7" "Verify FPGA Files" \
            "8" "Build pihpsdr" \
            "9" "Complete System Update" \
            "10" "Exit Program" 3>&1 1>&2 2>&3)

        case $choice in
            1) run_function "update_libraries" ;;
            2) run_function "update_repository" ;;
            3) run_function "build_p2app" ;;
            4) run_function "build_desktop_apps" ;;
            5) run_function "install_udev_rules" ;;
            6) run_function "copy_desktop_icons" ;;
            7) run_function "check_fpga_bit_file" ;;
            8) run_function "build_pihpsdr" ;;
            9) run_function "perform_all_tasks" ;;
            10) break ;;
            *) whiptail --msgbox "Invalid selection" 10 60 ;;
        esac
    done
}

#############################################################################
# COMMAND LINE PARSING
#############################################################################

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-git)
            SKIP_UPDATE=true
            shift
            ;;
        --skip-backup)
            SKIP_BACKUP=true
            shift
            ;;
        --force)
            FORCE_UPDATE=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --repo-url)
            SATURN_REPO_URL="$2"
            shift 2
            ;;
        --branch)
            SATURN_BRANCH="$2"
            shift 2
            ;;
        --rollback)
            rollback_changes
            exit $?
            ;;
        --help)
            show_usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            show_usage
            exit 1
            ;;
    esac
done

#############################################################################
# ERROR HANDLING SETUP
#############################################################################

# Trap for cleanup on script exit
cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo ""
        echo "Script failed with exit code: $exit_code"
        echo "Log file available at: $LOG_FILE"
        whiptail --title "Error" --msgbox "Script failed with exit code: $exit_code\nLog file available at: $LOG_FILE" 10 60
    fi
}

trap cleanup EXIT

#############################################################################
# MAIN SCRIPT EXECUTION
#############################################################################

echo "Starting Saturn Update Script v$SCRIPT_VERSION"
echo "Log file: $LOG_FILE"
echo "$(date): Update started"
whiptail --title "Saturn Update" --msgbox "Starting Saturn Update Script v$SCRIPT_VERSION\nLog file: $LOG_FILE" 10 60

# Check system requirements
check_system_requirements

# Check if Saturn directory exists
if [ ! -d "$SATURN_DIR" ]; then
    echo "✗ Error: Saturn directory not found at $SATURN_DIR"
    echo "Please ensure the Saturn repository is cloned at the expected location"
    whiptail --title "Critical Error" --msgbox "Saturn directory not found at:\n$SATURN_DIR" 10 60
    exit 1
fi

# Navigate to Saturn directory
cd "$SATURN_DIR" || exit 1
echo "✓ Working directory: $(pwd)"

# Launch the main GUI menu
main_menu

# Final system message after menu exit
print_header "Update Process Complete"
echo "Recommended actions:"
echo "1. Restart affected services"
echo "2. Check desktop shortcuts"
echo "3. Verify FPGA version if updated"
echo ""
echo "System will return to user prompt..."
cd "$HOME" || exit 1
