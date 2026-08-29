#!/usr/bin/env bash
#
# Compose mods/xmax3-tuned.json onto its workshop bases and install
# the results into the game's CustomMaps directory.
#
# The tuning loop this exists for: edit a number in the spec, run
# this, restart the game. Both bundles are always rebuilt together so
# they can never drift apart.
#
# Paths can be overridden from the environment, e.g.
#   TQ_CUSTOMMAPS=/tmp/try ./scripts/install-mods.sh
#
# Usage: ./scripts/install-mods.sh [path/to/spec.json]

set -euo pipefail

GAME_DIR="${TQ_GAME_DIR:-/Volumes/scott-games/steamapps/common/Titan Quest Anniversary Edition}"
WORKSHOP="${TQ_WORKSHOP:-/Volumes/scott-games/steamapps/workshop/content/475150/1779344333}"
CUSTOM_MAPS="${TQ_CUSTOMMAPS:-/Volumes/scott-games/tq1-saves/Titan Quest - Immortal Throne/CustomMaps}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="${1:-$REPO_ROOT/mods/xmax3-tuned.json}"

# base workshop folder -> installed bundle name
BUNDLES=(
    "LootPlusXMAXFTWx3:LootPlusXMAX3Tuned"
    "LootPlusXMAXFTWx3x1:LootPlusXMAX3Tuned1xBoss"
)

# A sample the tuning loop cares about: record path and the variable
# to read back once the bundles are in place.
PROBE_RECORD='records\effects\earth\volcanicorblobbed01.dbr'
PROBE_VARIABLE="projectileVelocity"

die() {
    echo "error: $*" >&2
    exit 1
}

[ -f "$SPEC" ] || die "patch spec not found: $SPEC"
[ -d "$GAME_DIR" ] || die "game dir not found: $GAME_DIR (is the games volume mounted?)"
[ -d "$WORKSHOP" ] || die "workshop dir not found: $WORKSHOP"
[ -d "$CUSTOM_MAPS" ] || die "CustomMaps not found: $CUSTOM_MAPS"

if pgrep -qf "Titan Quest" 2>/dev/null; then
    echo "warning: Titan Quest looks like it is running — it reads the" >&2
    echo "         database at load, so restart it after this finishes." >&2
fi

cd "$REPO_ROOT"
echo "building modforge…"
cargo build --release --quiet -p univault-core --example modforge --example dump

for entry in "${BUNDLES[@]}"; do
    base="${entry%%:*}"
    name="${entry##*:}"
    echo
    echo "=== $name (base: $base) ==="
    [ -d "$WORKSHOP/$base" ] || die "base mod not found: $WORKSHOP/$base"
    # modforge prints one line per patched variable; the summary is
    # the last line and the only part worth seeing by default.
    ./target/release/examples/modforge \
        "$GAME_DIR" "$WORKSHOP/$base" "$SPEC" "$CUSTOM_MAPS" "$name" | tail -1
done

echo
echo "=== installed $PROBE_VARIABLE on $PROBE_RECORD ==="
for entry in "${BUNDLES[@]}"; do
    name="${entry##*:}"
    value=$(./target/release/examples/dump \
        "$CUSTOM_MAPS/$name/database/$name.arz" "$PROBE_RECORD" |
        grep -i "^$PROBE_VARIABLE" || echo "(absent — left at vanilla)")
    echo "  $name: $value"
done

echo
echo "done — restart the game to load the new database."
