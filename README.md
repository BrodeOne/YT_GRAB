# YT Grab

YouTube video downloader for offline playback. Standalone installers for macOS and Windows — no prerequisites.

## Features

- Paste a URL → auto-fetch metadata, thumbnails, and all available formats
- **Combos** tab: pre-paired video + audio per resolution (MP4 recommended)
- **Audio** tab: audio-only download
- **Video** tab: video-only with auto-merge
- Live progress bar with speed, ETA, downloaded size
- Open File button after download completes
- Auto-fetch on paste (clipboard sniffing)

## Download

Releases page: https://github.com/BrodeOne/YT_GRAB/releases

- **Windows:** `YT Grab_x.x.x_x64.msi`
- **macOS:** `YT Grab_x.x.x_aarch64.dmg`

## Development

```bash
# Prerequisites: Node.js 22+, Rust stable
npm install

# Run in dev mode (requires yt-dlp and ffmpeg on PATH)
npm run tauri dev

# Build installer
npm run tauri build
```

## CI/CD

- **Build** (`build.yml`): CI on push to main — compiles and uploads artifacts
- **Release** (`release.yml`): Trigger manually or via `v*` tag — creates per-platform GitHub Releases

## Tech Stack

Tauri 2 (Rust) + React 19 + TypeScript. Powered by yt-dlp and ffmpeg (bundled in the installer).
