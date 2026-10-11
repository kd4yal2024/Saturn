#!/usr/bin/env bash
# The managed-release branch of update-saturn-go.sh (sourced by it; not meant to be run). When the installation has managed updates enabled, the
# Update action does not copy a binary into /opt/saturn-go: it resolves the selected GitHub ref to ONE commit, builds that commit into a release
# bundle on this device (saturn-release-build.sh, which never activates anything), and hands the bundle to the root-owned update broker
# (saturn-release-update-root.py), which runs the existing root installer and activator: install inactive, switch, restart, prove reception with a
# headless receive-only session, and restore the previous release if that fails. Nothing here installs, restarts or rolls back by itself.
#
# Variables and functions it relies on, from update-saturn-go.sh: DRY_RUN SKIP_GIT SKIP_BUILD SKIP_DEPLOY STAGE_ONLY VERBOSE MIGRATE_TO_RELEASES
# REPO_ROOT SATURNGO_URL SATURNGO_REF STATUS_PHASE and info warn die progress write_status ensure_public_policy_repo_accessible.
# saturn-update-contract: release-flow

# shellcheck disable=SC2034  # UPDATE_MODE and BROKER_OWNS_STATUS are read by write_status and the exit trap in update-saturn-go.sh
UPDATE_MODE="legacy"
BROKER_OWNS_STATUS=0

release_config_value(){   # release_config_value KEY -> the value in the root-owned activation configuration (last line wins), empty if unset
  local key="$1" file="${SATURN_RELEASE_ACTIVATE_CONFIG:-/etc/default/saturn-release-activate}" line value=""
  [[ -r "$file" ]] || return 0
  while IFS= read -r line; do
    [[ "$line" == "$key="* ]] && value="${line#*=}"
  done <"$file"
  value="${value#\"}"; value="${value%\"}"; value="${value#\'}"; value="${value%\'}"
  printf '%s' "$value"
}

managed_updates_enabled(){ [[ "$(release_config_value ACTIVATION_ENABLED)" == "1" ]]; }

release_active_commit(){  # the full commit of the active release, empty when no release is active (a legacy layout)
  local link target
  link="$(release_config_value CURRENT_LINK)"; link="${link:-/opt/saturn/current}"
  [[ -L "$link" ]] || return 0
  target="$(readlink -f "$link")"
  [[ "$(basename "$target")" =~ ^[0-9a-f]{40}$ ]] && basename "$target"
  return 0
}

run_release_flow(){
  local scripts_dir builder broker staging bundle resolved active manifest_tool components out line rc
  local -a migrate=() builder_args=() sudo_cmd=()
  UPDATE_MODE="release"
  scripts_dir="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
  broker="${SATURN_RELEASE_UPDATE_BROKER:-$scripts_dir/saturn-release-update-root.py}"
  manifest_tool="${SATURN_RELEASE_MANIFEST_TOOL:-$scripts_dir/saturn-release-manifest.py}"
  components="${SATURN_RELEASE_COMPONENTS_FILE:-/usr/local/lib/saturn-go/release/components-v1.json}"
  staging="${SATURN_RELEASE_STAGING_DIR:-/var/lib/saturn-state/release-staging}"
  builder="${SATURN_RELEASE_BUILDER:-$REPO_ROOT/update_manager/scripts/saturn-release-build.sh}"
  read -r -a sudo_cmd <<<"${SATURN_RELEASE_SUDO:-sudo -n}"
  (( ! MIGRATE_TO_RELEASES )) || migrate=(--allow-legacy-migration)

  info "Managed release update: this installs a verified release of the selected ref, proves reception itself with a temporary receive-only"
  info "headless session, and restores the previous release automatically if that fails. No browser has to be connected."
  STATUS_PHASE="preflight"
  write_status "running" "$STATUS_PHASE" "Checking that this installation can take a managed update"
  progress 5
  [[ -f "$broker" ]] || die "The managed-update broker is not installed ($broker). Re-run the installer (sudo ./install.sh) from this checkout."
  set +e
  out="$("${sudo_cmd[@]}" "$broker" --preflight "${migrate[@]}" 2>&1)"
  rc=$?
  set -e
  if (( rc != 0 )); then
    while IFS= read -r line; do info "$line"; done <<<"$out"
    die "This installation cannot take a managed update: ${out##*$'\n'}"
  fi
  info "$out"

  active="$(release_active_commit)"
  info "Active release: ${active:-none (legacy layout)}"
  [[ -x "$builder" ]] || die "The release builder is not executable: $builder"

  STATUS_PHASE="resolve"
  write_status "running" "$STATUS_PHASE" "Resolving the selected ref to one commit"
  progress 10
  if (( SKIP_GIT )); then
    info "Skipping the remote (--skip-git): building the active repo root as it is"
    if ! git -C "$REPO_ROOT" diff --quiet --ignore-submodules -- || ! git -C "$REPO_ROOT" diff --quiet --cached --ignore-submodules --; then
      die "The repo has uncommitted changes; a release is built from an exact clean commit"
    fi
    resolved="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  else
    [[ -n "$SATURNGO_URL" ]] || die "Saturn Go policy repo URL not configured. Save the Saturn Go page settings first."
    ensure_public_policy_repo_accessible "$SATURNGO_URL" "$SATURNGO_REF"
    builder_args=(--source-remote "$SATURNGO_URL" --source-ref "$SATURNGO_REF")
    out="$(SATURN_RELEASE_REPO_ROOT="$REPO_ROOT" GIT_TERMINAL_PROMPT=0 "$builder" "${builder_args[@]}" --resolve-only 2>&1)" \
      || die "The selected ref could not be resolved: ${out##*$'\n'}"
    resolved="$(printf '%s\n' "$out" | sed -n 's/^resolved_commit=//p' | head -1)"
  fi
  [[ "$resolved" =~ ^[0-9a-f]{40}$ ]] || die "The selected ref did not resolve to a full commit (got '${resolved:-nothing}')"
  info "Ref '${SATURNGO_REF}' resolves to commit $resolved"

  if [[ "$resolved" == "$active" ]]; then
    write_status "success" "done" "Release ${resolved:0:12} is already the active release; nothing to do" 1 0
    info "Release ${resolved:0:12} is already active. Nothing to do."
    progress 100
    info "Done"
    return 0
  fi
  if (( DRY_RUN )); then
    info "[dry-run] would build commit $resolved into $staging/$resolved, hand it to $broker --dispatch, and let the root activator switch to it"
    write_status "success" "dry-run" "Dry run: release ${resolved:0:12} would be built and activated" 1 0
    progress 100
    info "Done"
    return 0
  fi

  bundle="$staging/$resolved"
  STATUS_PHASE="build"
  if [[ -d "$bundle" ]]; then
    info "A built bundle for this commit already exists: $bundle"
  elif (( SKIP_BUILD )); then
    die "--skip-build was given but there is no built bundle for $resolved in $staging"
  else
    write_status "running" "$STATUS_PHASE" "Building release ${resolved:0:12} on this device (this takes a long time)"
    info "Building release ${resolved:0:12} (native build on this device; the radio keeps running)..."
    progress 20
    (
      export SATURN_RELEASE_OUTPUT_ROOT="$staging" SATURN_RELEASE_REPO_ROOT="$REPO_ROOT" GIT_TERMINAL_PROMPT=0
      "$builder" "${builder_args[@]}"
    ) || die "The release build failed (see the output above); nothing was installed or changed"
    [[ -d "$bundle" ]] || die "The build finished but did not produce $bundle"
  fi

  STATUS_PHASE="validate"
  write_status "running" "$STATUS_PHASE" "Validating the built release"
  progress 80
  if [[ -f "$manifest_tool" && -f "$components" ]]; then
    python3 "$manifest_tool" validate --release-root "$bundle" --components "$components" || die "The built release failed manifest validation; nothing was installed"
  else
    warn "Manifest tool or component policy not found here; the root installer will validate the bundle"
  fi

  if (( STAGE_ONLY || SKIP_DEPLOY )); then
    write_status "success" "stage" "Release ${resolved:0:12} built and validated; not installed (--stage-only / --skip-deploy)" 1 0
    info "Built and validated: $bundle (not installed)"
    progress 100
    info "Done"
    return 0
  fi

  STATUS_PHASE="dispatch"
  write_status "running" "$STATUS_PHASE" "Handing the release to the root update broker"
  progress 90
  # From here the broker owns the status file (it writes its own, more specific reason when it refuses) and runs the update in its own transient
  # unit, because activating restarts this service.
  BROKER_OWNS_STATUS=1
  "${sudo_cmd[@]}" "$broker" --dispatch "${migrate[@]}" "$bundle" || die "The update broker refused the release (see the message above); nothing was changed"
  progress 100
  info "The update is running in a detached unit. $SERVICE_NAME restarts during it: reload this page after a minute."
  info "The deploy status panel shows the outcome: success, or a rollback to the previous release with the reason."
  info "Done"
}
