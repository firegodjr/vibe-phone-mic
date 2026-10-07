#!/usr/bin/env bash
# Build dist/VibePhoneMic-x86_64.AppImage (release build + manual AppDir + appimagetool).
# Links the system libpulse / libGL / X11 / wayland dynamically.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
TOOLS="$ROOT/tools"
DIST="$ROOT/dist"
APPDIR="$HERE/target/AppDir"
APPIMAGETOOL="${APPIMAGETOOL:-$TOOLS/appimagetool}"
RUNTIME="${APPIMAGE_RUNTIME:-$TOOLS/runtime-x86_64}"
OUT="$DIST/VibePhoneMic-x86_64.AppImage"

# Keep cargo's registry/cache inside the repo unless the caller chose otherwise.
export CARGO_HOME="${CARGO_HOME:-$ROOT/.cache/cargo}"

echo "==> cargo build --release"
(cd "$HERE" && cargo build --release)
BIN="$HERE/target/release/vibe-phone-mic"

echo "==> AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
install -m755 "$BIN" "$APPDIR/usr/bin/vibe-phone-mic"
"$BIN" --render-icon "$APPDIR/vibe-phone-mic.png"
cp "$APPDIR/vibe-phone-mic.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/vibe-phone-mic.png"
ln -s vibe-phone-mic.png "$APPDIR/.DirIcon"

cat > "$APPDIR/vibe-phone-mic.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Vibe Phone Mic
GenericName=Phone Microphone
Comment=Use your Android phone as a microphone
Exec=vibe-phone-mic
Icon=vibe-phone-mic
Terminal=false
Categories=AudioVideo;Audio;
Keywords=microphone;phone;android;mic;
StartupWMClass=vibe-phone-mic
EOF
cp "$APPDIR/vibe-phone-mic.desktop" "$APPDIR/usr/share/applications/"

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/vibe-phone-mic" "$@"
EOF
chmod +x "$APPDIR/AppRun"

echo "==> appimagetool"
if [ ! -x "$APPIMAGETOOL" ]; then
    echo "appimagetool not found at $APPIMAGETOOL (set APPIMAGETOOL=...)" >&2
    exit 1
fi
if [ ! -s "$RUNTIME" ]; then
    echo "downloading AppImage runtime to $RUNTIME"
    curl -fsSL -o "$RUNTIME.part" https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
    mv "$RUNTIME.part" "$RUNTIME"
fi
mkdir -p "$DIST"
TOOL_ARGS=()
# Run appimagetool without FUSE if FUSE is unavailable.
if [ ! -e /dev/fuse ] || ! command -v fusermount >/dev/null 2>&1; then
    TOOL_ARGS+=(--appimage-extract-and-run)
fi
ARCH=x86_64 "$APPIMAGETOOL" "${TOOL_ARGS[@]}" --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "$OUT"
chmod +x "$OUT"
echo "==> built $OUT"
