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

### macOS: First Launch

The app is not code-signed. macOS Gatekeeper will block it on first launch.

**Fix:** Right-click `YT Grab.app` → **Open** → click **Open** in the dialog.

Alternatively, remove the quarantine flag:
```bash
xattr -d com.apple.quarantine "/Applications/YT Grab.app"
```

## Tech Stack

Tauri 2 (Rust) + React 19 + TypeScript. Bundled binaries: **yt-dlp**, **ffmpeg**, and **Deno** (Deno is yt-dlp's JavaScript runtime, required since yt-dlp 2025.11.12 for YouTube's JS challenges).

## How binaries are managed

`src-tauri/binaries/` is gitignored — binaries are downloaded at build time by `scripts/fetch-binaries.sh`:

- **Pinned versions** (`YTDLP_VERSION`, `DENO_VERSION`) and **SHA-256 verified** against the projects' published checksums. Bump the pins deliberately: yt-dlp must stay current because YouTube rotates its player clients; an outdated yt-dlp eventually gets `HTTP 403` on media downloads.
- On first launch the app stages yt-dlp into the user data dir (`%APPDATA%\com.ytgrab.desktop\bin` on Windows, `~/Library/Application Support/com.ytgrab.desktop/bin` on macOS) and self-updates it via `yt-dlp -U` (at most weekly). Deno is staged alongside it and passed explicitly via `--js-runtimes`.
- If a download fails with a YouTube client error (403 / "not a bot" / player response), the app automatically retries once with the `web_embedded` player client.

## Building the Windows installer (.msi)

### Via GitHub Actions (recommended)

1. Push to `main` → the **Build** workflow compiles and uploads per-platform artifacts (`.msi` for Windows).
2. Create a release:
   - Trigger the **Release** workflow manually (`Actions` → `Release` → `Run workflow`) with the version, e.g. `0.2.0`, **or**
   - Push a tag: `git tag v0.2.0 && git push origin v0.2.0`
3. The workflow downloads pinned, checksum-verified binaries (yt-dlp, ffmpeg, Deno), runs `npx tauri build`, and publishes `YT Grab_x.x.x_x64.msi` as a GitHub Release.

Requirements on GitHub: none — the runners fetch everything. WiX (for `.msi`) is fetched automatically by the Tauri bundler.

### Building locally on Windows

Prerequisites:

- Windows 10 or later (x64)
- [Git for Windows](https://git-scm.com/download/win) (provides the Bash shell and `sha256sum` used by the fetch script)
- [Node.js 22+](https://nodejs.org/) (with npm)
- Rust stable via [rustup](https://rustup.rs/) with the **MSVC toolchain** (default on Windows; requires the "Desktop development with C++" workload of Visual Studio Build Tools 2022 so `link.exe`/`rc.exe` are available)

Steps (in Git Bash):

```bash
git clone https://github.com/BrodeOne/YT_GRAB.git
cd YT_GRAB

npm install

# Downloads pinned, checksum-verified yt-dlp, ffmpeg, and Deno
bash scripts/fetch-binaries.sh

# Full release build (NSIS installer + MSI)
npm run tauri build
```

Output: `src-tauri/target/release/bundle/msi/YT Grab_x.x.x_x64.msi`

Install and run:

1. Double-click the `.msi` and complete the wizard.
2. Launch **YT Grab** from the Start Menu.
3. SmartScreen may warn because the app is unsigned → **More info** → **Run anyway**.

### Line endings

`.gitattributes` forces LF for shell scripts, Rust, and config files, so `fetch-binaries.sh` runs correctly under Git Bash on Windows CI regardless of `core.autocrlf` settings.

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
