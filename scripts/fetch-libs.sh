#!/bin/bash
# Fetches the 1.21.1 optional-integration jars into libs/ (not on a Maven repo,
# and gitignored). build.gradle compiles against them; Test.ps1 copies the two
# top-level jars into the GameTest run's mods/ folder.
#
#   libs/sable-neoforge-1.21.1-<v>.jar   Sable (physics sub-levels / Create Aeronautics)
#   libs/create-1.21.1-<v>.jar           Create (Redstone Link module)
#   libs/compile/*.jar                   jar-in-jar libraries of the two above,
#                                        extracted so javac can see them
#                                        (e.g. catnip's Couple lives in Ponder)
#
# Idempotent: skips any jar that is already present with the right SHA-1.
set -e

cd "$(dirname "$0")/.."

JARS=(
    "sable-neoforge-1.21.1-2.0.5.jar|https://cdn.modrinth.com/data/T9PomCSv/versions/U678xqle/sable-neoforge-1.21.1-2.0.5.jar|05f666e973d32baaaf405acb9bbed6615b909971"
    "create-1.21.1-6.0.10.jar|https://cdn.modrinth.com/data/LNytGWDc/versions/UjX6dr61/create-1.21.1-6.0.10.jar|0e97e49837bed766e6f28a4c95b04885d6acc353"
)

sha1_of() {
    if command -v sha1sum >/dev/null 2>&1; then
        sha1sum "$1" | cut -d' ' -f1
    else
        shasum -a 1 "$1" | cut -d' ' -f1
    fi
}

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --retry 3 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$2" "$1"
    else
        echo "Error: need curl or wget to download $1" >&2
        exit 1
    fi
}

mkdir -p libs/compile
for entry in "${JARS[@]}"; do
    IFS='|' read -r name url sha <<< "$entry"
    path="libs/$name"
    if [ -f "$path" ] && [ "$(sha1_of "$path")" = "$sha" ]; then
        continue
    fi
    echo "Fetching $name"
    download "$url" "$path"
    if [ "$(sha1_of "$path")" != "$sha" ]; then
        echo "Error: SHA-1 mismatch for $path" >&2
        rm -f "$path"
        exit 1
    fi
done

# Extract nested jar-in-jar libraries for the compiler. unzip -j flattens
# META-INF/jarjar/ into libs/compile/.
for entry in "${JARS[@]}"; do
    IFS='|' read -r name _ _ <<< "$entry"
    unzip -j -o -q "libs/$name" 'META-INF/jarjar/*.jar' -d libs/compile 2>/dev/null || true
done
