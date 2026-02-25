#!/usr/bin/env bash
set -euo pipefail

APP_PATH="${1:-./p3app}"
RUN_SECONDS="${RUN_SECONDS:-3}"
SHUTDOWN_TIMEOUT_SECONDS="${SHUTDOWN_TIMEOUT_SECONDS:-10}"
LOG_FILE="${LOG_FILE:-/tmp/p3app_shutdown_smoke.log}"

if [[ ! -x "${APP_PATH}" ]]; then
  echo "[ERROR] App binary not executable: ${APP_PATH}" >&2
  exit 2
fi

echo "[INFO] Starting shutdown smoke test"
echo "[INFO] App: ${APP_PATH}"
echo "[INFO] Log: ${LOG_FILE}"
echo "[INFO] Run window: ${RUN_SECONDS}s"
echo "[INFO] Shutdown timeout: ${SHUTDOWN_TIMEOUT_SECONDS}s"

"${APP_PATH}" -s >"${LOG_FILE}" 2>&1 &
APP_PID=$!
echo "[INFO] Spawned PID ${APP_PID}"

sleep "${RUN_SECONDS}"

if ! kill -0 "${APP_PID}" 2>/dev/null; then
  echo "[ERROR] Process exited before shutdown signal. See ${LOG_FILE}" >&2
  if wait "${APP_PID}"; then
    EXIT_CODE=0
  else
    EXIT_CODE=$?
  fi
  echo "[ERROR] Process exited with code ${EXIT_CODE}. See ${LOG_FILE}" >&2
  exit "${EXIT_CODE}"
fi

echo "[INFO] Sending SIGINT"
kill -INT "${APP_PID}"

for ((i = 0; i < SHUTDOWN_TIMEOUT_SECONDS * 10; i++)); do
  if ! kill -0 "${APP_PID}" 2>/dev/null; then
    if wait "${APP_PID}"; then
      EXIT_CODE=0
    else
      EXIT_CODE=$?
    fi
    if [[ ${EXIT_CODE} -eq 0 ]]; then
      echo "[OK] Process exited cleanly"
      exit 0
    fi
    echo "[ERROR] Process exited with code ${EXIT_CODE}. See ${LOG_FILE}" >&2
    exit "${EXIT_CODE}"
  fi
  sleep 0.1
done

echo "[ERROR] Process did not exit within timeout; sending SIGKILL" >&2
kill -KILL "${APP_PID}" 2>/dev/null || true
wait "${APP_PID}" || true
echo "[ERROR] Shutdown smoke failed. See ${LOG_FILE}" >&2
exit 1
