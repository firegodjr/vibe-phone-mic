#!/usr/bin/env bash
# Only needed on systems whose PipeWire install lacks audio support (e.g. a
# minimal Arch container: no audioconvert / protocol-pulse / wireplumber).
# Downloads the matching Arch packages and extracts them (no install, no root)
# into <repo>/.cache/pw-private/root, which tools/pw-private.sh picks up.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${VPM_PW_ROOT:-$HERE/../../.cache/pw-private/root}"
PKGS="$(dirname "$DEST")/pkgs"
mkdir -p "$DEST" "$PKGS"
command -v pacman >/dev/null || { echo "not an Arch system; install your distro's pipewire audio packages instead" >&2; exit 1; }
for name in pipewire-audio pipewire-pulse wireplumber libwireplumber; do
    ver="$(pacman -Si "$name" | awk -F': ' '/^Version/{print $2; exit}')"
    f="$PKGS/$name-$ver-x86_64.pkg.tar.zst"
    [ -s "$f" ] || curl -fsSL -o "$f" "https://geo.mirror.pkgbuild.com/extra/os/x86_64/$name-$ver-x86_64.pkg.tar.zst"
    tar --zstd -xf "$f" -C "$DEST"
done
echo "extracted into $DEST"
