#!/usr/bin/env bash
# Build the desktop distribution artifacts for one platform.
#
#   package.sh <linux|macos|windows> <suffix> <tag> <desktop-bin> <engine-bin> <outdir>
#
# Every bundle carries `torrentng-desktop` plus the matching `torrentngd`
# (the same binary published as the Server download) so Local mode works
# out of the box.
set -euo pipefail

kind="${1:?kind}"; suffix="${2:?suffix}"; tag="${3:?tag}"
desktop_bin="${4:?desktop binary}"; engine_bin="${5:?engine binary}"; out="${6:?outdir}"
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

# v20261012-3 / v1.2.3 -> 20261012.3 / 1.2.3 (same rule as the PPA).
version="${tag#main-}"; version="${version#v}"; version="${version//-/.}"
name="torrentng-desktop-${tag}-${suffix}"

case "$kind" in
  linux)
    d="$stage/$name"
    mkdir -p "$d"
    install -m755 "$desktop_bin" "$d/torrentng-desktop"
    install -m755 "$engine_bin" "$d/torrentngd"
    install -m644 "$here/torrentng-desktop.desktop" "$here/torrentng-desktop.svg" "$here/README.txt" "$d/"
    tar -czf "$out/$name.tar.gz" -C "$stage" "$name"

    case "$suffix" in
      linux-x86_64) arch=amd64 ;;
      linux-aarch64) arch=arm64 ;;
      *) echo "no deb architecture for $suffix" >&2; exit 1 ;;
    esac
    root="$stage/deb"
    install -Dm755 "$desktop_bin" "$root/usr/bin/torrentng-desktop"
    install -Dm755 "$engine_bin" "$root/usr/lib/torrentng-desktop/torrentngd"
    install -Dm644 "$here/torrentng-desktop.desktop" "$root/usr/share/applications/torrentng-desktop.desktop"
    install -Dm644 "$here/torrentng-desktop.svg" "$root/usr/share/icons/hicolor/scalable/apps/torrentng-desktop.svg"
    install -Dm644 "$here/README.txt" "$root/usr/share/doc/torrentng-desktop/README.txt"
    mkdir -p "$root/DEBIAN"
    cat > "$root/DEBIAN/control" <<CONTROL
Package: torrentng-desktop
Version: ${version}
Section: net
Priority: optional
Architecture: ${arch}
Maintainer: TorrentNG <noreply@users.noreply.github.com>
Depends: libc6, libgl1 | libegl1, libxkbcommon0
Recommends: xdg-desktop-portal
Description: TorrentNG Desktop - native torrent client
 Native desktop client for the TorrentNG engine. Includes a private copy of
 torrentngd for local use and can also connect to a remote TorrentNG server.
CONTROL
    dpkg-deb --root-owner-group --build "$root" "$out/torrentng-desktop_${version}_${arch}.deb" >/dev/null
    ;;

  macos)
    app="$stage/TorrentNG.app"
    mkdir -p "$app/Contents/MacOS"
    install -m755 "$desktop_bin" "$app/Contents/MacOS/torrentng-desktop"
    install -m755 "$engine_bin" "$app/Contents/MacOS/torrentngd"
    sed "s/@VERSION@/${version}/g" "$here/Info.plist.in" > "$app/Contents/Info.plist"
    # Ad-hoc signature so the bundle is internally consistent; this is NOT
    # notarisation (documented in docs/DESKTOP.md).
    codesign --force --deep --sign - "$app" || echo "codesign unavailable; shipping unsigned" >&2
    hdiutil create -volname TorrentNG -srcfolder "$app" -ov -format UDZO "$out/$name.dmg" >/dev/null
    ;;

  windows)
    d="$stage/$name"
    mkdir -p "$d"
    install -m755 "$desktop_bin" "$d/TorrentNG.exe"
    install -m755 "$engine_bin" "$d/torrentngd.exe"
    install -m644 "$here/README.txt" "$d/README.txt"
    src="$d"; dst="$out/$name.zip"
    if command -v cygpath >/dev/null 2>&1; then src="$(cygpath -w "$d")"; dst="$(cygpath -w "$dst")"; fi
    powershell -NoProfile -Command "Compress-Archive -Path '$src' -DestinationPath '$dst' -Force"
    ;;

  *) echo "unknown kind: $kind" >&2; exit 2 ;;
esac
ls -l "$out"
