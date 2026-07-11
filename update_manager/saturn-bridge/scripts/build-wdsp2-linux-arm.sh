#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BRIDGE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

NATIVE_SOURCE_ROOT="${SATURN_BRIDGE_NATIVE_SOURCE_ROOT:-${BRIDGE_DIR}/target/native-src}"
WDSP2_SOURCE_DIR="${WDSP2_SOURCE_DIR:-${NATIVE_SOURCE_ROOT}/OpenHPSDR-wdsp/wdsp 2.00/Source}"
PIHPSDR_WDSP_DIR="${PIHPSDR_WDSP_DIR:-${NATIVE_SOURCE_ROOT}/pihpsdr/wdsp}"
BUILD_DIR="${WDSP2_BUILD_DIR:-${BRIDGE_DIR}/target/wdsp2-linux-arm}"

if [[ ! -d "${WDSP2_SOURCE_DIR}" ]]; then
  echo "ERROR: WDSP 2.00 source directory not found: ${WDSP2_SOURCE_DIR}" >&2
  exit 1
fi
if [[ ! -f "${PIHPSDR_WDSP_DIR}/linux_port.c" || ! -f "${PIHPSDR_WDSP_DIR}/linux_port.h" ]]; then
  echo "ERROR: piHPSDR linux_port sources not found under: ${PIHPSDR_WDSP_DIR}" >&2
  exit 1
fi
if ! command -v pkg-config >/dev/null 2>&1; then
  echo "ERROR: pkg-config is required" >&2
  exit 1
fi
if ! pkg-config --exists fftw3; then
  echo "ERROR: fftw3 development package is required" >&2
  exit 1
fi

rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"
cp -a "${WDSP2_SOURCE_DIR}/." "${BUILD_DIR}/"
cp "${PIHPSDR_WDSP_DIR}/linux_port.c" "${PIHPSDR_WDSP_DIR}/linux_port.h" "${BUILD_DIR}/"

python3 - "${BUILD_DIR}" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])

comm = root / "comm.h"
text = comm.read_text()
text = text.replace(
    "#include <Windows.h>\n#include <process.h>\n#include <intrin.h>\n",
    "#if defined(linux) || defined(__APPLE__)\n"
    "#include <stdlib.h>\n"
    "#include <pthread.h>\n"
    "#include <semaphore.h>\n"
    "#include <string.h>\n"
    "#include \"linux_port.h\"\n"
    "#endif\n\n"
    "#ifdef _WIN32\n"
    "#include <Windows.h>\n"
    "#include <process.h>\n"
    "#include <intrin.h>\n"
    "#endif\n",
)
text = text.replace(
    "#include <time.h>\n#include <avrt.h>\n#include <assert.h>\n#include \"fftw3.h\"\n",
    "#include <time.h>\n"
    "#include <limits.h>\n"
    "#ifdef _WIN32\n"
    "#include <avrt.h>\n"
    "#endif\n"
    "#include <assert.h>\n"
    "#include \"fftw3.h\"\n\n"
    "#if defined(linux) || defined(__APPLE__)\n"
    "#define dprintf wdsp_dprintf\n"
    "#endif\n",
)
comm.write_text(text)

linux_port_h = root / "linux_port.h"
text = linux_port_h.read_text()
text = text.replace("#define CRITICAL_SECTION pthread_mutex_t\n", "#define CRITICAL_SECTION pthread_mutex_t\n#define LPCRITICAL_SECTION pthread_mutex_t *\n")
text = text.replace(
    "#define freopen_s freopen\n",
    "int LinuxFreopenS(FILE **stream, const char *filename, const char *mode, FILE *old_stream);\n"
    "#define freopen_s LinuxFreopenS\n",
)
text = text.replace(
    "#define THREAD_PRIORITY_HIGHEST 0\n",
    "#define THREAD_PRIORITY_HIGHEST 0\n"
    "#define _MM_FLUSH_ZERO_ON 0\n"
    "#define _MM_SET_FLUSH_ZERO_MODE(x) ((void)0)\n"
    "#define AvSetMmThreadCharacteristics(a,b) ((HANDLE)0)\n"
    "#define AvSetMmThreadPriority(a,b) ((void)0)\n"
    "#define AvRevertMmThreadCharacteristics(a) ((void)0)\n"
    "#define GetCurrentThread() ((HANDLE)0)\n"
    "#define OutputDebugStringA(x) ((void)0)\n"
    "#define AllocConsole() ((void)0)\n"
    "#define FreeConsole() ((void)0)\n",
)
text = text.replace("#define CreateSemaphore(a,b,c,d) LinuxCreateSemaphore(a,b,c,d)\n", "#define CreateSemaphore(a,b,c,d) LinuxCreateSemaphore(a,b,c,d)\n#define CreateSemaphoreW(a,b,c,d) LinuxCreateSemaphore(a,b,c,d)\n")
text = text.replace("#define WaitForSingleObject(x, y) LinuxWaitForSingleObject(x, y)\n", "#define WaitForSingleObject(x, y) LinuxWaitForSingleObject(x, y)\n#define WaitForMultipleObjects(a,b,c,d) LinuxWaitForMultipleObjects(a,b,c,d)\n")
text = text.replace("#define INFINITE -1\n", "#define INFINITE -1\n#define WAIT_OBJECT_0 0\n")
text = text.replace("int LinuxWaitForSingleObject(sem_t *sem,int x);\n", "int LinuxWaitForSingleObject(sem_t *sem,int x);\nint LinuxWaitForMultipleObjects(int count, void **sems, int wait_all, int ms);\n")
linux_port_h.write_text(text)

linux_port_c = root / "linux_port.c"
text = linux_port_c.read_text()
old_wait_for_single_object = (
    "int LinuxWaitForSingleObject(sem_t *sem,int ms) {\n"
    "\tint result=0;\n"
    "\tif(ms==INFINITE) {\n"
    "\t\t// wait for the lock\n"
    "\t\tresult=sem_wait(sem);\n"
    "\t} else {\n"
    "\t\tfor (int i = 0; i < ms; i++) {\n"
    "\t\t  result=sem_trywait(sem);\n"
    "\t\t  if (result == 0) break;\n"
    "\t\t  Sleep(1);\n"
    "\t\t}\n"
    "\t}\n\n"
    "\treturn result;\n"
    "}\n"
)
new_wait_for_single_object = (
    "int LinuxWaitForSingleObject(sem_t *sem,int ms) {\n"
    "\tif (ms == INFINITE) return sem_wait(sem);\n\n"
    "\tint result = sem_trywait(sem);\n"
    "\tfor (int elapsed = 0; result != 0 && elapsed < ms; elapsed++) {\n"
    "\t\tSleep(1);\n"
    "\t\tresult = sem_trywait(sem);\n"
    "\t}\n"
    "\treturn result;\n"
    "}\n"
)
if old_wait_for_single_object not in text:
    raise RuntimeError("piHPSDR LinuxWaitForSingleObject implementation changed")
text = text.replace(old_wait_for_single_object, new_wait_for_single_object, 1)
text = text.replace(
    "#if defined(linux) || defined(__APPLE__)\n\n",
    "#if defined(linux) || defined(__APPLE__)\n\n"
    "int LinuxFreopenS(FILE **stream, const char *filename, const char *mode, FILE *old_stream) {\n"
    "\tFILE *result = freopen(filename, mode, old_stream);\n"
    "\tif (stream != NULL) *stream = result;\n"
    "\treturn result == NULL;\n"
    "}\n\n",
    1,
)
text = text.replace(
    "\t} else\tif (start_address == &doPSCalcCorrection\n"
    "\t\t\t || start_address == &doPSTurnoff\n"
    "\t\t || start_address == &PSSaveCorrection\n"
    "\t\t || start_address == &PSRestoreCorrection) {\n",
    "\t} else\tif (start_address == &doPSCorrChange) {\n",
)
text = text.replace(
    "\treturn result;\n}\n\nsem_t *LinuxCreateSemaphore",
    "\treturn result;\n}\n\n"
    "int LinuxWaitForMultipleObjects(int count, void **sems, int wait_all, int ms) {\n"
    "\tif (wait_all) return -1;\n"
    "\tint elapsed = 0;\n"
    "\tfor (;;) {\n"
    "\t\tfor (int i = 0; i < count; i++) {\n"
    "\t\t\tif (sem_trywait((sem_t *)sems[i]) == 0) return WAIT_OBJECT_0 + i;\n"
    "\t\t}\n"
    "\t\tif (ms != INFINITE && elapsed >= ms) return -1;\n"
    "\t\tSleep(1);\n"
    "\t\tif (ms != INFINITE) elapsed++;\n"
    "\t}\n"
    "}\n\n"
    "sem_t *LinuxCreateSemaphore",
)
linux_port_c.write_text(text)

snoop = root / "snoop.c"
if snoop.exists():
    snoop.write_text(snoop.read_text().replace("void xsnoop(channel)\n", "void xsnoop(int channel)\n"))

wbfm = root / "wbfm.c"
text = wbfm.read_text()
old_dmph_update = (
    "\t\ta->dmph = dmph_run;\n"
    "\t\ta->dmph_type = dmph_continent;\n"
    "\t\tLeaveCriticalSection(&ch[channel].csDSP);\n"
)
new_dmph_update = (
    "\t\ta->dmph = dmph_run;\n"
    "\t\ta->dmph_type = dmph_continent;\n"
    "\t\ta->dmphL->run = dmph_run;\n"
    "\t\ta->dmphR->run = dmph_run;\n"
    "\t\ta->dmphL->tau = dmph_continent ? 50.0e-6 : 75.0e-6;\n"
    "\t\ta->dmphR->tau = dmph_continent ? 50.0e-6 : 75.0e-6;\n"
    "\t\tcalc_dmph(a->dmphL);\n"
    "\t\tcalc_dmph(a->dmphR);\n"
    "\t\tLeaveCriticalSection(&ch[channel].csDSP);\n"
)
if old_dmph_update not in text:
    raise RuntimeError("WDSP 2.00 SetRXAWBFMdmph implementation changed")
wbfm.write_text(text.replace(old_dmph_update, new_dmph_update, 1))
PY

cd "${BUILD_DIR}"
mapfile -t sources < <(find . -maxdepth 1 -name '*.c' -printf '%f\n' | sort)
cflags=(-pthread -O3 -D_GNU_SOURCE -Wno-parentheses -Wcast-align)
if [[ -d "${PIHPSDR_WDSP_DIR}/../rnnoise/include" ]]; then
  cflags+=("-I${PIHPSDR_WDSP_DIR}/../rnnoise/include")
fi
if [[ -d "${PIHPSDR_WDSP_DIR}/../libspecbleach/include" ]]; then
  cflags+=("-I${PIHPSDR_WDSP_DIR}/../libspecbleach/include")
fi
read -r -a fftw_cflags <<<"$(pkg-config --cflags fftw3)"
cflags+=("${fftw_cflags[@]}")

for source in "${sources[@]}"; do
  cc "${cflags[@]}" -c -o "${source%.c}.o" "${source}"
done

ar rcs libwdsp.a ./*.o
ranlib libwdsp.a

echo "Built WDSP 2.00 Linux/ARM archive:"
echo "  ${BUILD_DIR}/libwdsp.a"
echo
echo "Build Saturn Bridge against it with:"
echo "  SATURN_WDSP_DIR=${BUILD_DIR} cargo build --release"
