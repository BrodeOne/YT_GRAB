#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$SCRIPT_DIR/../src-tauri/binaries"

# Pinned versions. Bump deliberately: yt-dlp must stay current because YouTube
# rotates its player clients; Deno is yt-dlp's JS challenge runtime.
YTDLP_VERSION="2026.08.19"
DENO_VERSION="v2.9.0"

mkdir -p "$BIN_DIR"

if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
  TARGET="win"
elif [[ "$OSTYPE" == "darwin"* ]]; then
  TARGET="mac"
else
  TARGET="linux"
fi

echo "Target: $TARGET"

sha256_of() {
  local dir="$1" filename="$2"
  (
    cd "$dir"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "$filename" | awk '{print $1}'
    else
      shasum -a 256 "$filename" | awk '{print $1}'
    fi
  )
}

verify_sha256() {
  local dir="$1" sums_file="$2" filename="$3"
  local expected
  expected=$(grep -F " $filename" "$sums_file" | awk '{print $1}' | head -1)
  if [ -z "$expected" ]; then
    echo "FAIL: no checksum for $filename in $sums_file"
    exit 1
  fi
  local actual
  actual=$(sha256_of "$dir" "$filename")
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: checksum mismatch for $filename"
    echo "  expected: $expected"
    echo "  actual:   $actual"
    exit 1
  fi
  echo "  $filename checksum OK"
}

extract_zip() {
  # $1 = zip path, $2 = destination dir (created if needed)
  local zip="$1" dest="$2"
  mkdir -p "$dest"
  if [[ "$TARGET" == "win" ]]; then
    # Git Bash on Windows runners/local machines may not ship `unzip`.
    # PowerShell Expand-Archive is always available on Windows.
    local wzip wdest
    wzip=$(cygpath -w "$zip")
    wdest=$(cygpath -w "$dest")
    powershell.exe -NoProfile -NonInteractive -Command "Expand-Archive -LiteralPath \"$wzip\" -DestinationPath \"$wdest\" -Force"
  else
    unzip -o "$zip" -d "$dest"
  fi
}

if [[ "$TARGET" == "win" || "$1" == "all" ]]; then
  echo "=== Downloading Windows binaries ==="
  curl --retry 3 -sSfL -o "$BIN_DIR/yt-dlp.exe" "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp.exe"
  curl --retry 3 -sSfL -o /tmp/ytdlp.SUMS "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/SHA2-256SUMS"
  verify_sha256 "$BIN_DIR" /tmp/ytdlp.SUMS "yt-dlp.exe"
  rm -f /tmp/ytdlp.SUMS
  echo "  yt-dlp.exe downloaded"

  if [ ! -f "$BIN_DIR/ffmpeg.exe" ]; then
    FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    curl --retry 5 -sSfL -o /tmp/ffmpeg_win.zip "$FFMPEG_URL"
    extract_zip /tmp/ffmpeg_win.zip /tmp/ffmpeg_win
    mv /tmp/ffmpeg_win/ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe "$BIN_DIR/ffmpeg.exe"
    rm -rf /tmp/ffmpeg_win /tmp/ffmpeg_win.zip
  fi
  echo "  ffmpeg.exe ready"

  DENO_ASSET="deno-x86_64-pc-windows-msvc.zip"
  curl --retry 3 -sSfL -o "/tmp/${DENO_ASSET}" "https://github.com/denoland/deno/releases/download/${DENO_VERSION}/${DENO_ASSET}"
  curl --retry 3 -sSfL -o "/tmp/${DENO_ASSET}.sha256sum" "https://github.com/denoland/deno/releases/download/${DENO_VERSION}/${DENO_ASSET}.sha256sum"
  verify_sha256 /tmp "/tmp/${DENO_ASSET}.sha256sum" "$DENO_ASSET"
  extract_zip "/tmp/${DENO_ASSET}" "$BIN_DIR"
  rm -f "/tmp/${DENO_ASSET}" "/tmp/${DENO_ASSET}.sha256sum"
  echo "  deno.exe downloaded"
fi

if [[ "$TARGET" == "mac" || "$1" == "all" ]]; then
  echo "=== Downloading macOS binaries ==="
  curl --retry 3 -sSfL -o "$BIN_DIR/yt-dlp_macos" "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_macos"
  curl --retry 3 -sSfL -o /tmp/ytdlp.SUMS "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/SHA2-256SUMS"
  verify_sha256 "$BIN_DIR" /tmp/ytdlp.SUMS "yt-dlp_macos"
  rm -f /tmp/ytdlp.SUMS
  chmod +x "$BIN_DIR/yt-dlp_macos"
  cp "$BIN_DIR/yt-dlp_macos" "$BIN_DIR/yt-dlp"
  echo "  yt-dlp_macos downloaded"

  if [ ! -f "$BIN_DIR/ffmpeg" ]; then
    curl --retry 5 -sSfL -o /tmp/ffmpeg_mac.zip "https://evermeet.cx/ffmpeg/getrelease/zip"
    extract_zip /tmp/ffmpeg_mac.zip /tmp/ffmpeg_mac
    mv /tmp/ffmpeg_mac/ffmpeg "$BIN_DIR/ffmpeg"
    chmod +x "$BIN_DIR/ffmpeg"
    rm -rf /tmp/ffmpeg_mac /tmp/ffmpeg_mac.zip
  fi
  echo "  ffmpeg ready"

  DENO_ARCH="aarch64"
  if [[ "$(uname -m)" == "x86_64" ]]; then
    DENO_ARCH="x86_64"
  fi
  DENO_ASSET="deno-${DENO_ARCH}-apple-darwin.zip"
  curl --retry 3 -sSfL -o "/tmp/${DENO_ASSET}" "https://github.com/denoland/deno/releases/download/${DENO_VERSION}/${DENO_ASSET}"
  curl --retry 3 -sSfL -o "/tmp/${DENO_ASSET}.sha256sum" "https://github.com/denoland/deno/releases/download/${DENO_VERSION}/${DENO_ASSET}.sha256sum"
  verify_sha256 /tmp "/tmp/${DENO_ASSET}.sha256sum" "$DENO_ASSET"
  extract_zip "/tmp/${DENO_ASSET}" /tmp/deno_mac
  mv /tmp/deno_mac/deno "$BIN_DIR/deno"
  chmod +x "$BIN_DIR/deno"
  rm -rf /tmp/deno_mac "/tmp/${DENO_ASSET}" "/tmp/${DENO_ASSET}.sha256sum"
  echo "  deno downloaded"
fi

echo "=== Done ==="
ls -lh "$BIN_DIR/"
