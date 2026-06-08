#!/usr/bin/env bash
# ============================================================================
# build-android.sh — empaca un crate Llimphi-Android como APK firmado.
#
# Uso:
#   ./build-android.sh <crate-dir> [arch] [profile]
#
#   crate-dir : path al Cargo.toml del crate Android (cdylib + android_main)
#   arch      : arm64 | x64           (default arm64)
#   profile   : release | debug       (default release)
#
# Requisitos:
#   - rustup target add aarch64-linux-android x86_64-linux-android
#   - cargo install xbuild       (binario `x`)
#   - cargo install cargo-ndk    (opcional, sólo si querés build sin APK)
#   - NDK r27+ en $ANDROID_NDK_HOME
#   - Android SDK en $ANDROID_HOME (cmdline-tools + build-tools)
#   - PEM dev en $LLIMPHI_PEM (se crea automáticamente la primera vez)
#
# Resultado:
#   target/x/<profile>/android/<crate>.apk   — APK firmado v2, instalable con
#                                              `adb install -r <apk>`.
# ============================================================================
set -euo pipefail

CRATE_DIR="${1:?se requiere crate-dir como primer argumento}"
ARCH="${2:-arm64}"
PROFILE="${3:-release}"

# --- toolchain -------------------------------------------------------------
: "${ANDROID_NDK_HOME:=/home/sergio/android-ndk-r27c}"
: "${ANDROID_NDK_ROOT:=$ANDROID_NDK_HOME}"
: "${ANDROID_HOME:=/opt/android-sdk}"
: "${LLIMPHI_PEM:=$HOME/.local/share/llimphi-android/debug.pem}"
export ANDROID_NDK_HOME ANDROID_NDK_ROOT ANDROID_HOME

X_BIN="${X_BIN:-$HOME/.cargo/bin/x}"
test -x "$X_BIN" || { echo "❌ xbuild (cargo install xbuild)"; exit 1; }
test -d "$ANDROID_NDK_HOME" || { echo "❌ NDK no encontrado en $ANDROID_NDK_HOME"; exit 1; }
test -d "$ANDROID_HOME"     || { echo "❌ SDK no encontrado en $ANDROID_HOME"; exit 1; }

# --- PEM de firma dev (RSA 2048 + cert auto-firmado) -----------------------
if [ ! -f "$LLIMPHI_PEM" ]; then
    echo "→ generando PEM de firma dev en $LLIMPHI_PEM"
    mkdir -p "$(dirname "$LLIMPHI_PEM")"
    openssl req -x509 -newkey rsa:2048 \
        -keyout "${LLIMPHI_PEM}.key" \
        -out    "${LLIMPHI_PEM}.cert" \
        -days 36500 -nodes \
        -subj "/CN=llimphi-dev/O=tawasuyu/C=AR" 2>/dev/null
    cat "${LLIMPHI_PEM}.key" "${LLIMPHI_PEM}.cert" > "$LLIMPHI_PEM"
fi

# --- flags -----------------------------------------------------------------
PROFILE_FLAG="--release"
[ "$PROFILE" = "debug" ] && PROFILE_FLAG="--debug"

# --- build ----------------------------------------------------------------
cd "$CRATE_DIR"
CRATE_NAME=$(grep '^name *=' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
echo "→ building $CRATE_NAME · $ARCH · $PROFILE"

"$X_BIN" build \
    --platform android \
    --arch "$ARCH" \
    --format apk \
    $PROFILE_FLAG \
    --pem "$LLIMPHI_PEM"

# --- locate + verify -------------------------------------------------------
APK=$(find ../../../../target/x/$PROFILE/android -name "${CRATE_NAME}.apk" 2>/dev/null | head -1)
[ -z "$APK" ] && APK=$(find . -name "${CRATE_NAME}.apk" 2>/dev/null | head -1)
[ -z "$APK" ] && { echo "❌ APK no encontrado"; exit 1; }
APK=$(readlink -f "$APK")
SIZE=$(du -h "$APK" | cut -f1)

APKSIGNER="$ANDROID_HOME/build-tools/37.0.0/apksigner"
if [ -x "$APKSIGNER" ]; then
    if "$APKSIGNER" verify --min-sdk-version 24 "$APK" 2>/dev/null; then
        echo "✓ firma verificada (APK Signature Scheme v2)"
    else
        echo "⚠ firma no verifica"
    fi
fi

echo "✓ $APK ($SIZE)"
echo
echo "Instalar en device:"
echo "  adb install -r $APK"
