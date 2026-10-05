#!/bin/sh
# Live production Planet view acceptance for issue #57. Never reuse evidence.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
mode=acceptance
if [ "${1:-}" = --diagnostic ]; then
    mode=diagnostic
    shift
fi
output=${1:-/tmp/terra-planet-57}
case "$output" in /*) ;; *) output="$PWD/$output" ;; esac
if [ -e "$output" ] || [ -L "$output" ]; then
    echo "Choose a new output directory; preserving existing evidence: $output" >&2
    exit 1
fi
parent=$(CDPATH= cd -- "$(dirname -- "$output")" && pwd -P) || {
    echo "The output parent directory must already exist" >&2; exit 1;
}
output="$parent/$(basename -- "$output")"
case "$output/" in "$root/"*)
    echo "Choose an output directory outside the checkout" >&2; exit 1 ;;
esac
assets=${TERRA_PLANET_ASSET_ROOT:-$root/crates/main}
assets=$(CDPATH= cd -- "$assets" && pwd -P)
mkdir "$output"
cd "$root"
unset TERRA_PRODUCTION_CAPTURE TERRA_ASSET_SHOWCASE TERRA_ASSET_CAPTURE TERRA_FLIGHT_CAPTURE
unset TERRA_LIGHTING_CAPTURE TERRA_LIGHTING_SCENE TERRA_LIGHTING_PHASE TERRA_LIGHTING_PROBE
export BEVY_ASSET_ROOT="$assets"
unset TERRA_PLANET_ACCEPTANCE_CAPTURE TERRA_PLANET_TRANSITION_DIAGNOSTIC
if [ "$mode" = diagnostic ]; then
    export TERRA_PLANET_TRANSITION_DIAGNOSTIC="$output"
else
    export TERRA_PLANET_ACCEPTANCE_CAPTURE="$output"
fi
TERRA_SOURCE_REVISION=$(git rev-parse HEAD)
TERRA_SOURCE_BRANCH=$(git branch --show-current)
export TERRA_SOURCE_REVISION TERRA_SOURCE_BRANCH
git rev-parse HEAD > "$output/revision.txt"
git status --porcelain=v1 > "$output/worktree-status.txt"
git diff HEAD --binary > "$output/source.patch"
python3 - "$assets" "$output" <<'PY'
import hashlib
import os
from pathlib import Path
import subprocess
import sys
assets, output = map(Path, sys.argv[1:])

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()

files = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'])
with (output / 'source-hashes.txt').open('w') as result:
    for name in sorted(set(os.fsdecode(name) for name in files.split(b'\0') if name)):
        path = Path(name)
        if path.is_file():
            result.write(f'{digest(path)}  {name}\n')
        elif path.is_symlink():
            result.write(f'symlink:{os.readlink(path)}  {name}\n')
with (output / 'hashes.txt').open('w') as result:
    for directory, dirs, names in os.walk(assets / 'assets', followlinks=True):
        dirs.sort()
        for name in sorted(names):
            path = Path(directory) / name
            if path.is_file():
                relative = path.relative_to(assets / 'assets')
                result.write(f'{digest(path)}  crates/main/assets/{relative}\n')
(output / 'asset-root.txt').write_text(str(assets) + '\n')
PY
sw_vers > "$output/os.txt"
system_profiler SPHardwareDataType SPDisplaysDataType > "$output/hardware-displays.txt"
rustc --version --verbose > "$output/rustc.txt"
# Build before querying metadata so any build failure preserves its exact status.
capture_features=asset-review
if [ "${TERRA_PLANET_SCHEDULE_TRACE+x}" = x ]; then
  capture_features=asset-review-schedule-trace
  printf '%s\n' 'Schedule tracing is diagnostic instrumentation; repeat final acceptance without it.' > "$output/profile-caveat.txt"
fi
cargo build --locked -p main --features "$capture_features" > "$output/build.log" 2>&1
cargo --version > "$output/cargo.txt"
cargo metadata --locked --no-deps --format-version 1 > "$output/cargo-metadata.json"
target=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["target_directory"])' "$output/cargo-metadata.json")
printf '%s\n' "cargo build --locked -p main --features $capture_features" > "$output/build-command.txt"
"$target/debug/main" > "$output/app.log" 2>&1
if [ "$mode" = diagnostic ]; then
    python3 - "$output" <<'PYTHON'
from pathlib import Path
import sys
output = Path(sys.argv[1])
status = output / 'diagnostic-status.txt'
expected = 'status=diagnostic-complete'
if not status.is_file() or expected not in status.read_text().splitlines():
    sys.exit(f'Diagnostic incomplete or rejected; inspect {output}')
print(f'Diagnostic complete in {output}; rendered inspection is still required.')
PYTHON
else
    python3 "$root/scripts/validate_planet_capture.py" "$output"
fi
