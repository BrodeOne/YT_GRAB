#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$SCRIPT_DIR/../src-tauri/binaries"

mkdir -p "$BIN_DIR"

if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
  TARGET="win"
elif [[ "$OSTYPE" == "darwin"* ]]; then
  TARGET="mac"
else
  TARGET="linux"
fi

echo "Target: $TARGET"

if [[ "$TARGET" == "win" || "$1" == "all" ]]; then
  echo "=== Downloading Windows binaries ==="
  if [ ! -f "$BIN_DIR/yt-dlp.exe" ]; then
    curl -sSfL -o "$BIN_DIR/yt-dlp.exe" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    echo "  yt-dlp.exe downloaded"
  fi
  if [ ! -f "$BIN_DIR/ffmpeg.exe" ]; then
    FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    curl -sSfL -o /tmp/ffmpeg_win.zip "$FFMPEG_URL"
    unzip -o /tmp/ffmpeg_win.zip "ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe" -d /tmp/ffmpeg_win
    mv /tmp/ffmpeg_win/ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe "$BIN_DIR/ffmpeg.exe"
    rm -rf /tmp/ffmpeg_win /tmp/ffmpeg_win.zip
    echo "  ffmpeg.exe downloaded"
  fi
fi

if [[ "$TARGET" == "mac" || "$1" == "all" ]]; then
  echo "=== Downloading macOS binaries ==="
  if [ ! -f "$BIN_DIR/yt-dlp_macos" ]; then
    curl -sSfL -o "$BIN_DIR/yt-dlp_macos" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    chmod +x "$BIN_DIR/yt-dlp_macos"
    echo "  yt-dlp_macos downloaded"
  fi
  if [ ! -f "$BIN_DIR/ffmpeg" ]; then
    curl -sSfL -o /tmp/ffmpeg_mac.zip "https://evermeet.cx/ffmpeg/getrelease/zip"
    unzip -o /tmp/ffmpeg_mac.zip -d /tmp/ffmpeg_mac
    mv /tmp/ffmpeg_mac/ffmpeg "$BIN_DIR/ffmpeg"
    chmod +x "$BIN_DIR/ffmpeg"
    rm -rf /tmp/ffmpeg_mac /tmp/ffmpeg_mac.zip
    echo "  ffmpeg downloaded for macOS"
  fi
fi

echo "=== Done ==="
ls -lh "$BIN_DIR/"
