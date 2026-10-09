#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# The provisioner has a source guard, so loading its build helper has no
# installation side effects. Run its user commands in this isolated fixture.
# shellcheck source=../provision/cloud-init/provision-saturn.sh
source "$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
run_as_user() { shift; "$@"; }

project="$TMP_DIR/audiotest"
mkdir -p "$project"
printf 'int main(void) { return 0; }\n' >"$project/audiotest.c"
printf 'all: audiotest\naudiotest: audiotest.o\n\t$(CC) -o $@ $^\naudiotest.o: audiotest.c\n\t$(CC) -c -o $@ $<\n' >"$project/Makefile"

# Both files are newer than the source. A plain make would accept the empty
# executable; deleting only the empty build outputs must force a real build.
: >"$project/audiotest.o"
: >"$project/audiotest"
touch "$project/audiotest.o" "$project/audiotest"
printf 'keep\n' >"$project/operator-notes.txt"
build_dir "$TMP_DIR" audiotest "$project" 1 1
[[ -s "$project/audiotest.o" && -s "$project/audiotest" ]]
"$project/audiotest"
grep -Fxq keep "$project/operator-notes.txt"

# Match the observed G2 failure: a valid executable but a newer empty object
# would otherwise be linked and produce "undefined reference to main".
: >"$project/audiotest.o"
touch "$project/audiotest.o"
build_dir "$TMP_DIR" audiotest "$project" 1 1
[[ -s "$project/audiotest.o" && -s "$project/audiotest" ]]
"$project/audiotest"

printf 'Saturn empty-build-artifact recovery tests passed\n'
