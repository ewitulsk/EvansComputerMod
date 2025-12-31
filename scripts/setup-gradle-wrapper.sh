#!/bin/bash
#
# Downloads the Gradle wrapper JAR if it doesn't exist.
# This script should be run before using ./gradlew for the first time.
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
WRAPPER_DIR="$PROJECT_ROOT/gradle/wrapper"
WRAPPER_JAR="$WRAPPER_DIR/gradle-wrapper.jar"
WRAPPER_PROPERTIES="$WRAPPER_DIR/gradle-wrapper.properties"

# Gradle wrapper JAR download URL (matches Gradle 8.10.2)
GRADLE_VERSION="8.10.2"
WRAPPER_JAR_URL="https://raw.githubusercontent.com/gradle/gradle/v${GRADLE_VERSION}/gradle/wrapper/gradle-wrapper.jar"

# Alternative: Use the Gradle services URL
WRAPPER_JAR_URL_ALT="https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-wrapper.jar.zip"

echo "Setting up Gradle wrapper..."

# Create wrapper directory if it doesn't exist
mkdir -p "$WRAPPER_DIR"

# Check if wrapper JAR already exists
if [ -f "$WRAPPER_JAR" ]; then
    echo "Gradle wrapper JAR already exists at $WRAPPER_JAR"
    exit 0
fi

# Check if properties file exists
if [ ! -f "$WRAPPER_PROPERTIES" ]; then
    echo "Error: gradle-wrapper.properties not found at $WRAPPER_PROPERTIES"
    exit 1
fi

echo "Downloading Gradle wrapper JAR..."

# Try downloading from GitHub first
if curl -fsSL -o "$WRAPPER_JAR" "$WRAPPER_JAR_URL" 2>/dev/null; then
    echo "Successfully downloaded Gradle wrapper JAR from GitHub"
elif curl -fsSL -o "$WRAPPER_JAR" "https://github.com/gradle/gradle/raw/v${GRADLE_VERSION}/gradle/wrapper/gradle-wrapper.jar" 2>/dev/null; then
    echo "Successfully downloaded Gradle wrapper JAR from GitHub (alternative URL)"
else
    # Fallback: Download from Maven Central
    MAVEN_URL="https://repo1.maven.org/maven2/org/gradle/gradle-wrapper/8.10.2/gradle-wrapper-8.10.2.jar"
    if curl -fsSL -o "$WRAPPER_JAR" "$MAVEN_URL" 2>/dev/null; then
        echo "Successfully downloaded Gradle wrapper JAR from Maven Central"
    else
        echo "Error: Failed to download Gradle wrapper JAR"
        echo "Please download it manually from: https://services.gradle.org/distributions/"
        exit 1
    fi
fi

# Verify the JAR was downloaded
if [ -f "$WRAPPER_JAR" ] && [ -s "$WRAPPER_JAR" ]; then
    echo "Gradle wrapper setup complete!"
else
    echo "Error: Downloaded file is empty or missing"
    rm -f "$WRAPPER_JAR"
    exit 1
fi
