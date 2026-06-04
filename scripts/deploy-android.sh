#!/usr/bin/env bash
#
# deploy-android.sh — Build, install, and launch Terax on a connected
#                     Android device or emulator.
#
# Usage:
#   ./scripts/deploy-android.sh              # auto-detect device, build+install+launch
#   ./scripts/deploy-android.sh --no-launch   # skip `am start`
#   ./scripts/deploy-android.sh --uninstall   # remove app first (fixes INSTALL_FAILED_UPDATE_INCOMPATIBLE)
#   ./scripts/deploy-android.sh --serial 1234 # target specific device serial
#
# Requirements:
#   - Android SDK / platform-tools on PATH (adb)
#   - Android NDK (for Rust .so compilation via the Tauri build plugin)
#   - Java 17+ (JVM target 17)
#   - gradlew at android/gradlew

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GRADLEW="$PROJECT_ROOT/android/gradlew"
APK="$PROJECT_ROOT/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
PACKAGE="app.crynta.terax"
ACTIVITY=".MainActivity"

NO_LAUNCH=false
UNINSTALL=false
SERIAL=""

# --- Parse flags ----------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-launch)  NO_LAUNCH=true; shift ;;
        --uninstall)  UNINSTALL=true; shift ;;
        --serial)     SERIAL="$2";    shift 2 ;;
        *)            echo "Unknown flag: $1"; exit 1 ;;
    esac
done

ADB=(adb)
if [[ -n "$SERIAL" ]]; then
    ADB=(adb -s "$SERIAL")
fi

# --- Step 0: Check Gradle wrapper -----------------------------------------

if [[ ! -x "$GRADLEW" ]]; then
    echo "==> Fixing gradlew permissions (chmod +x)"
    chmod +x "$GRADLEW"
fi

# --- Step 1: Check ADB device ---------------------------------------------

echo "==> Checking ADB devices"
DEVICES=$("${ADB[@]}" devices 2>/dev/null | awk 'NR>1 && $2=="device" {print $1}')

if [[ -z "$DEVICES" ]]; then
    cat <<EOF

ERROR: No ADB device found.

Troubleshooting:
  1. Connect your device via USB and enable USB debugging:
     Settings → Developer options → USB debugging
  2. If Developer options is hidden, tap "About phone" → "Build number"
     7 times.
  3. Run: adb kill-server && adb start-server && adb devices
  4. On some devices you must approve the RSA key fingerprint
     on the device screen.
  5. For emulators: start one with:
     avdmanager list avd
     emulator -avd <name>
EOF
    exit 1
fi

COUNT=$(echo "$DEVICES" | wc -l | tr -d ' ')
if [[ "$COUNT" -gt 1 && -z "$SERIAL" ]]; then
    echo "ERROR: Multiple devices connected. Use --serial to pick one:"
    echo "$DEVICES"
    exit 1
fi

TARGET=$(echo "$DEVICES" | head -1)
echo "   Target device: $TARGET"

# --- Step 2: Uninstall (optional) -----------------------------------------

if $UNINSTALL; then
    echo "==> Uninstalling $PACKAGE"
    "${ADB[@]}" uninstall "$PACKAGE" 2>/dev/null || true
fi

# --- Step 3: Build debug APK ----------------------------------------------

echo "==> Building debug APK (assembleDebug)"
echo "    Working directory: $PROJECT_ROOT"
echo "    Gradle wrapper:    $GRADLEW"

if ! "$GRADLEW" -p "$PROJECT_ROOT/android" assembleDebug; then
    cat <<EOF

ERROR: Gradle build failed.

Common causes:
  - Missing Android SDK: set ANDROID_HOME or sdk.dir in local.properties
  - Missing Android NDK: the Tauri Rust .so build requires NDK;
    install via SDK Manager or set ndk.dir
  - Java version mismatch: this project targets JVM 17;
    run: java -version
  - Rust/NDK toolchain issue: run:
    cd src-tauri && cargo build --target aarch64-linux-android
EOF
    exit 1
fi

if [[ ! -f "$APK" ]]; then
    echo "ERROR: APK not found at $APK"
    ls "$PROJECT_ROOT/android/app/build/outputs/apk/debug/" 2>/dev/null || true
    exit 1
fi

APK_SIZE=$(stat -f%z "$APK" 2>/dev/null || stat -c%s "$APK" 2>/dev/null || echo "?")
echo "   APK size: $APK_SIZE bytes"

# --- Step 4: Install APK --------------------------------------------------

echo "==> Installing APK (adb install -r)"
INSTALL_OUTPUT=$("${ADB[@]}" install -r "$APK" 2>&1) || true

if echo "$INSTALL_OUTPUT" | grep -qi "success"; then
    echo "   Install succeeded."
elif echo "$INSTALL_OUTPUT" | grep -qi "INSTALL_FAILED_UPDATE_INCOMPATIBLE"; then
    cat <<EOF

ERROR: INSTALL_FAILED_UPDATE_INCOMPATIBLE

The installed version has a different signature than the APK you are
trying to install.  This happens when you switch between debug/release
builds, or between different machines.

Fix: run with --uninstall to remove the old version first:
  ./scripts/deploy-android.sh --uninstall
EOF
    exit 1
elif echo "$INSTALL_OUTPUT" | grep -qi "INSTALL_FAILED_ALREADY_EXISTS"; then
    echo "   App already installed. Re-running with -r."
    "${ADB[@]}" install -r "$APK" 2>&1
elif echo "$INSTALL_OUTPUT" | grep -qi "error"; then
    echo "ERROR: Install failed:"
    echo "$INSTALL_OUTPUT"
    exit 1
else
    echo "$INSTALL_OUTPUT"
fi

# --- Step 5: Launch activity (optional) -----------------------------------

if ! $NO_LAUNCH; then
    echo "==> Launching $PACKAGE/$ACTIVITY"
    "${ADB[@]}" shell am start -n "$PACKAGE/$ACTIVITY" 2>&1 || {
        echo "WARNING: am start failed (activity may already be running)"
    }
fi

echo ""
echo "Done. APK installed on $TARGET."
