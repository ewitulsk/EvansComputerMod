#!/bin/bash
# Builds the mod and copies the output jar to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

./gradlew build

JAR=$(find build/libs -name "*.jar" ! -name "*-sources.jar" ! -name "*-javadoc.jar" | head -1)

if [ -z "$JAR" ]; then
    echo "Error: No jar found in build/libs/"
    exit 1
fi

cp "$JAR" .
echo "Copied $(basename "$JAR") to project root"
