#!/bin/sh
# Frozen production-renderer baselines and one-variable probes for issue #31.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
output=${1:-/tmp/terra-lighting-31}
case "$output" in /*) ;; *) output="$PWD/$output" ;; esac
if [ -e "$output" ]; then
    echo "Choose a new output directory; preserving existing evidence: $output" >&2
    exit 1
fi
# Resolve the parent physically so symlinks and ../ cannot bypass this guard.
parent=$(CDPATH= cd -- "$(dirname -- "$output")" && pwd -P) || {
    echo "The output parent directory must already exist" >&2; exit 1;
}
output="$parent/$(basename -- "$output")"
case "$output/" in "$root/"*)
    echo "Choose an output directory outside the checkout" >&2
    exit 1
esac
mkdir "$output"
cd "$root"
unset TERRA_PRODUCTION_CAPTURE TERRA_ASSET_SHOWCASE TERRA_ASSET_CAPTURE TERRA_FLIGHT_CAPTURE
unset TERRA_LIGHTING_SCENE TERRA_LIGHTING_PHASE TERRA_LIGHTING_PROBE
export BEVY_ASSET_ROOT="$root/crates/main"
git rev-parse HEAD > "$output/revision.txt"
# HEAD alone does not identify a development checkout or ignored generated assets.
git status --porcelain=v1 > "$output/worktree-status.txt"
git diff HEAD --binary > "$output/source.patch"
git ls-files --cached --others --exclude-standard -z | xargs -0 shasum -a 256 > "$output/source-hashes.txt"
find crates/main/assets -type f -exec shasum -a 256 {} + | LC_ALL=C sort > "$output/hashes.txt"
cargo build --locked -p main --features asset-review > "$output/build.log" 2>&1
capture() {
    export TERRA_LIGHTING_CAPTURE="$output/$1"
    echo "Capturing $1"
    target/debug/main > "$output/$1.log" 2>&1
    expected=1
    if [ "$1" = baseline ]; then expected=15; fi
    count=0
    for png in "$TERRA_LIGHTING_CAPTURE"/*.png; do
        [ -s "$png" ] || { echo "Missing capture: $png" >&2; exit 1; }
        count=$((count + 1))
    done
    [ "$count" -eq "$expected" ] || { echo "Incomplete captures for $1" >&2; exit 1; }
    [ "$(wc -l < "$TERRA_LIGHTING_CAPTURE/measurements.txt")" -eq "$expected" ] || {
        echo "Incomplete measurements for $1" >&2; exit 1;
    }
}
capture baseline
export TERRA_LIGHTING_SCENE=dense-transition TERRA_LIGHTING_PHASE=noon
for probe in shadows no-fog no-atmosphere no-ambient exposure; do
    export TERRA_LIGHTING_PROBE="$probe"
    capture "$probe"
done
export TERRA_LIGHTING_SCENE=shoreline TERRA_LIGHTING_PHASE=sunset
for probe in rough-water flat-water no-fog no-atmosphere; do
    export TERRA_LIGHTING_PROBE="$probe"
    capture "water-$probe"
done
export TERRA_LIGHTING_SCENE=overview TERRA_LIGHTING_PHASE=noon
for probe in no-fog no-atmosphere; do
    export TERRA_LIGHTING_PROBE="$probe"
    capture "overview-$probe"
done
export TERRA_LIGHTING_SCENE=dense-transition TERRA_LIGHTING_PHASE=sunset TERRA_LIGHTING_PROBE=baseline
for run in 1 2; do
    capture "repeat-$run"
done
echo "Evidence saved in $output"
