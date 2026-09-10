use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

const APP_IDENTIFIER: &str = "com.ytgrab.desktop";

fn silent_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut std_cmd = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std_cmd.creation_flags(0x08000000);
    }

    let in_user_dir = user_bin_dir()
        .map(|ubd| {
            let abs = std::path::absolute(program).unwrap_or_else(|_| std::path::PathBuf::from(program));
            abs.starts_with(&ubd)
        })
        .unwrap_or(false);

    if !in_user_dir {
        if let Some(parent) = std::path::Path::new(program).parent() {
            if parent.as_os_str().len() > 0 {
                let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
                let existing = std::env::var_os("PATH").unwrap_or_default();
                let mut combined = std::ffi::OsString::from(parent);
                combined.push(sep);
                combined.push(&existing);
                std_cmd.env("PATH", combined);
            }
        }
    }

    Command::from(std_cmd)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VideoMetadata {
    pub id: String,
    pub title: String,
    pub duration: f64,
    pub thumbnail: String,
    pub webpage_url: String,
    pub formats: Vec<FormatInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FormatInfo {
    pub format_id: String,
    pub ext: String,
    pub resolution: String,
    pub vcodec: String,
    pub acodec: String,
    pub filesize: Option<u64>,
    pub fps: Option<f64>,
    pub tbr: Option<f64>,
    pub has_audio: bool,
    pub has_video: bool,
    pub format_note: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProgressPayload {
    pub id: String,
    pub percent: f64,
    pub speed: String,
    pub eta: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompletePayload {
    pub id: String,
    pub path: String,
    pub filename: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ErrorPayload {
    pub id: String,
    pub error: String,
}

fn parse_progress_line(line: &str) -> Option<(u64, Option<u64>, Option<f64>, Option<f64>, Option<f64>)> {
    let mut downloaded = None;
    let mut total = None;
    let mut speed = None;
    let mut eta = None;
    let mut yt_pct = None;

    for part in line.split('|') {
        let (key, val) = part.split_once(':')?;
        match key {
            "downloaded" => downloaded = val.parse::<u64>().ok(),
            "total" => total = val.parse::<u64>().ok(),
            "speed" => speed = val.parse::<f64>().ok(),
            "eta" => eta = val.parse::<f64>().ok(),
            "pct" => {
                yt_pct = val
                    .trim()
                    .trim_end_matches('%')
                    .parse::<f64>()
                    .ok()
                    .filter(|&p| p > 0.0 && p <= 100.0);
            }
            _ => {}
        }
    }

    downloaded.map(|d| (d, total, speed, eta, yt_pct))
}

fn calculate_percent(downloaded: u64, total: Option<u64>, yt_pct: Option<f64>) -> f64 {
    match total {
        Some(t) if t > 0 => {
            let p = (downloaded as f64 / t as f64) * 100.0;
            p.min(100.0)
        }
        _ => yt_pct.unwrap_or(0.0),
    }
}

struct ProcessManager {
    active_downloads: Arc<Mutex<HashMap<String, tokio::process::Child>>>,
    cancelled_downloads: Arc<Mutex<HashSet<String>>>,
}

fn user_bin_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support").join(APP_IDENTIFIER).join("bin"));
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::PathBuf::from(a).join(APP_IDENTIFIER).join("bin"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".local/share").join(APP_IDENTIFIER).join("bin"));
    base
}

fn user_exe_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

fn user_bin_path(name: &str) -> Option<std::path::PathBuf> {
    user_bin_dir().map(|dir| dir.join(user_exe_name(name)))
}

fn bundled_binary_path(name: &str) -> Option<std::path::PathBuf> {
    let exe_name = user_exe_name(name);
    let mut fallbacks: Vec<String> = vec![exe_name];
    if name == "yt-dlp" {
        fallbacks.push("yt-dlp_macos".to_string());
    }

    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    let mut roots: Vec<std::path::PathBuf> = vec![exe_dir.join("binaries")];
    if let Some(grandparent) = exe_dir.parent() {
        roots.push(grandparent.join("Resources").join("binaries"));
    }

    for root in roots {
        for f in &fallbacks {
            let candidate = root.join(f);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

async fn ytdlp_version(path: &std::path::Path) -> Option<String> {
    let output = silent_command(&path.to_string_lossy())
        .arg("--version")
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

async fn stage_bundled_file(bundled: &str, path: &std::path::Path) -> bool {
    let tmp = path.with_extension("tmp");
    if let Err(e) = tokio::fs::copy(bundled, &tmp).await {
        eprintln!("Could not stage {}: {}", path.display(), e);
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await;
    }
    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        if e.kind() == std::io::ErrorKind::AlreadyExists || e.raw_os_error() == Some(80) {
            let _ = tokio::fs::remove_file(path).await;
            if tokio::fs::rename(&tmp, path).await.is_ok() {
                return true;
            }
        }
        eprintln!("Could not replace {}: {}", path.display(), e);
        let _ = tokio::fs::remove_file(&tmp).await;
        return false;
    }
    true
}

async fn stage_deno_next_to(dir: &std::path::Path) {
    let Some(bundled_deno) = bundled_binary_path("deno") else {
        return;
    };
    let target = dir.join(user_exe_name("deno"));
    if tokio::fs::metadata(&target).await.is_ok() {
        return;
    }
    let _ = stage_bundled_file(&bundled_deno.to_string_lossy(), &target).await;
}

const UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

static STAGING_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn ensure_user_ytdlp(bundled: &str) -> Option<(String, String)> {
    let _guard = STAGING_LOCK.lock().await;

    let path = user_bin_path("yt-dlp")?;
    let dir = path.parent()?;

    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        eprintln!("Could not create {}: {}", dir.display(), e);
        return None;
    }

    let bundled_newer = match (
        tokio::fs::metadata(bundled).await,
        tokio::fs::metadata(&path).await,
    ) {
        (Ok(b), Ok(u)) => match (b.modified(), u.modified()) {
            (Ok(bm), Ok(um)) => bm > um,
            _ => true,
        },
        (Ok(_), Err(_)) => true,
        _ => false,
    };

    let stale = match tokio::fs::metadata(&path).await {
        Ok(meta) => match meta.modified() {
            Ok(mtime) => match std::time::SystemTime::now().duration_since(mtime) {
                Ok(age) => age > std::time::Duration::from_secs(14 * 24 * 60 * 60),
                Err(_) => true,
            },
            Err(_) => true,
        },
        Err(_) => true,
    };

    if stale || bundled_newer {
        if !stage_bundled_file(bundled, &path).await {
            return None;
        }
    }

    let mut version = match ytdlp_version(&path).await {
        Some(v) => v,
        None => {
            eprintln!("Staged yt-dlp failed validation; removing {}", path.display());
            let _ = tokio::fs::remove_file(&path).await;
            return None;
        }
    };

    let update_marker = dir.join(".last_update_check");
    let update_due = match tokio::fs::metadata(&update_marker).await {
        Ok(meta) => match meta.modified() {
            Ok(mtime) => match std::time::SystemTime::now().duration_since(mtime) {
                Ok(age) => age > UPDATE_CHECK_INTERVAL,
                Err(_) => true,
            },
            Err(_) => true,
        },
        Err(_) => true,
    };

    if update_due {
        let update = silent_command(&path.to_string_lossy())
            .arg("-U")
            .output();
        if let Ok(dur) = tokio::time::timeout(std::time::Duration::from_secs(120), update).await {
            if let Ok(out) = dur {
                if out.status.success() {
                    match ytdlp_version(&path).await {
                        Some(v) => version = v,
                        None => {
                            eprintln!("Updated yt-dlp failed validation; removing {}", path.display());
                            let _ = tokio::fs::remove_file(&path).await;
                            return None;
                        }
                    }
                }
            }
        }
        let _ = tokio::fs::write(&update_marker, b"").await;
    }

    stage_deno_next_to(dir).await;
    Some((path.to_string_lossy().to_string(), version))
}

fn find_binary(name: &str) -> String {
    let env_var = match name {
        "yt-dlp" => "YT_DLP_PATH",
        "ffmpeg" => "FFMPEG_PATH",
        _ => "YT_GRAB_BIN_PATH",
    };
    if let Ok(path) = std::env::var(env_var) {
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }

    if name == "yt-dlp" || name == "deno" {
        if let Some(candidate) = user_bin_path(name) {
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }

    if let Some(bundled) = bundled_binary_path(name) {
        return bundled.to_string_lossy().to_string();
    }

    let search_paths = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/opt/local/bin",
        "/usr/bin",
    ];

    for dir in &search_paths {
        let candidate = format!("{}/{}", dir, name);
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let candidate = format!("{}/.local/bin/{}", home, name);
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
        if let Ok(homebrew) = std::env::var("HOMEBREW_PREFIX") {
            let candidate = format!("{}/bin/{}", homebrew, name);
            if std::path::Path::new(&candidate).exists() {
                return candidate;
            }
        }
    }

    name.to_string()
}

#[tauri::command]
async fn check_binaries() -> Result<String, String> {
    let mut version = String::new();
    if let Some(bundled_path) = find_bundled_ytdlp() {
        if let Some((_path, v)) = ensure_user_ytdlp(&bundled_path).await {
            version = format!(" ({})", v);
        }
    }

    let yt = find_binary("yt-dlp");
    let ff = find_binary("ffmpeg");
    if yt.ends_with("yt-dlp") && !std::path::Path::new(&yt).exists() {
        return Err(format!(
            "yt-dlp not found. The app needs yt-dlp to download videos.\n\n\
            Expected locations:\n\
            - Bundled in the app (binaries/yt-dlp)\n\
            - /opt/homebrew/bin/yt-dlp\n\
            - /usr/local/bin/yt-dlp\n\n\
            Install with: brew install yt-dlp"
        ));
    }
    if ff.ends_with("ffmpeg") && !std::path::Path::new(&ff).exists() {
        return Err(format!(
            "ffmpeg not found. The app needs ffmpeg to merge video and audio.\n\n\
            Expected locations:\n\
            - Bundled in the app (binaries/ffmpeg)\n\
            - /opt/homebrew/bin/ffmpeg\n\
            - /usr/local/bin/ffmpeg\n\n\
            Install with: brew install ffmpeg"
        ));
    }

    Ok(format!("yt-dlp: {}{}\nffmpeg: {}", yt, version, ff))
}

fn find_bundled_ytdlp() -> Option<String> {
    bundled_binary_path("yt-dlp").map(|p| p.to_string_lossy().to_string())
}

fn js_runtime_args() -> Vec<String> {
    let deno = find_binary("deno");
    if std::path::Path::new(&deno).exists() {
        vec!["--js-runtimes".to_string(), format!("deno:{}", deno)]
    } else {
        Vec::new()
    }
}

#[tauri::command]
async fn fetch_metadata(url: String) -> Result<VideoMetadata, String> {
    let yt_dlp = find_binary("yt-dlp");

    let mut args = vec![
        "-J".to_string(),
        "--no-playlist".to_string(),
        "--flat-playlist".to_string(),
        "--no-check-certificate".to_string(),
        "--no-update".to_string(),
    ];
    args.extend(js_runtime_args());
    args.push(url.clone());

    let output = silent_command(&yt_dlp)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp error: {}", stderr));
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse metadata JSON: {}", e))?;

    let id = raw["id"].as_str().unwrap_or("").to_string();
    let title = raw["title"].as_str().unwrap_or("Unknown").to_string();
    let duration = raw["duration"].as_f64().unwrap_or(0.0);
    let thumbnail = raw["thumbnail"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let webpage_url = raw["webpage_url"].as_str().unwrap_or(&url).to_string();

    let formats: Vec<FormatInfo> = raw["formats"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|f| {
            let vcodec = f["vcodec"].as_str().unwrap_or("none").to_string();
            let acodec = f["acodec"].as_str().unwrap_or("none").to_string();
            let resolution = f["resolution"].as_str().unwrap_or("?").to_string();
            let format_note = f["format_note"].as_str().unwrap_or("").to_string();

            if format_note == "storyboard"
                || format_note.contains("m3u8")
            {
                return None;
            }

            Some(FormatInfo {
                format_id: f["format_id"].as_str().unwrap_or("").to_string(),
                ext: f["ext"].as_str().unwrap_or("?").to_string(),
                resolution,
                vcodec: vcodec.clone(),
                acodec: acodec.clone(),
                filesize: f["filesize"].as_u64(),
                fps: f["fps"].as_f64(),
                tbr: f["tbr"].as_f64(),
                has_audio: acodec != "none",
                has_video: vcodec != "none",
                format_note,
            })
        })
        .collect();

    Ok(VideoMetadata {
        id,
        title,
        duration,
        thumbnail,
        webpage_url,
        formats,
    })
}

fn is_youtube_client_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("sign in to confirm")
        || lower.contains("not a bot")
        || lower.contains("player response")
        || lower.contains("needs to be reloaded")
}

fn ytdlp_download_args(
    format_id: &str,
    output_template: &str,
    ffmpeg: &str,
    extra_args: &[&str],
    url: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-f".to_string(),
        format_id.to_string(),
        "-o".to_string(),
        output_template.to_string(),
        "--newline".to_string(),
        "--progress-template".to_string(),
        "downloaded:%(progress.downloaded_bytes)s|total:%(progress.total_bytes_estimate)s|speed:%(progress.speed)s|eta:%(progress.eta)s|pct:%(progress._percent_str)s".to_string(),
        "--ffmpeg-location".to_string(),
        ffmpeg.to_string(),
        "--no-playlist".to_string(),
        "--no-mtime".to_string(),
        "--no-update".to_string(),
    ];
    args.extend(extra_args.iter().map(|s| s.to_string()));
    args.extend(js_runtime_args());
    args.push(url.to_string());
    args
}

#[allow(clippy::too_many_arguments)]
async fn run_download_process(
    app: AppHandle,
    active: Arc<Mutex<HashMap<String, tokio::process::Child>>>,
    cancelled: Arc<Mutex<HashSet<String>>>,
    id: &str,
    yt_dlp: &str,
    ffmpeg: &str,
    url: &str,
    format_id: &str,
    output_template: &str,
    output_dir: &str,
    extra_args: &[&str],
) -> Result<(), String> {
    let args = ytdlp_download_args(format_id, output_template, ffmpeg, extra_args, url);

    let mut child = silent_command(yt_dlp)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {}", e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    active.lock().await.insert(id.to_string(), child);

    let stderr_reader = BufReader::new(stderr);
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut reader = stderr_reader;
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });

    let mut reader = BufReader::new(stdout).lines();

    let start_time = Instant::now();
    let mut last_bytes: u64 = 0;
    let mut last_stats_time = Instant::now();
    let mut smoothed_speed: f64 = 0.0;
    let mut last_emit_time = Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap_or(Instant::now());
    let mut last_emitted_percent: f64 = -1.0;
    let mut speed_initialized = false;
    const EARLY_PHASE_SECS: f64 = 3.0;
    const SPEED_ALPHA: f64 = 0.3;

    while let Ok(Some(line)) = reader.next_line().await {
        if let Some((downloaded, total, _raw_speed, _raw_eta, yt_pct)) = parse_progress_line(&line) {
            let now = Instant::now();
            let elapsed = now.duration_since(start_time).as_secs_f64();
            let percent = calculate_percent(downloaded, total, yt_pct);

            if downloaded > 0 && speed_initialized {
                let delta_bytes = downloaded.saturating_sub(last_bytes);
                let delta_time = now.duration_since(last_stats_time).as_secs_f64();

                if delta_time > 0.0 {
                    let inst_speed = delta_bytes as f64 / delta_time;
                    if smoothed_speed == 0.0 {
                        smoothed_speed = inst_speed;
                    } else {
                        smoothed_speed = SPEED_ALPHA * inst_speed + (1.0 - SPEED_ALPHA) * smoothed_speed;
                    }
                }

                last_bytes = downloaded;
                last_stats_time = now;
            } else if downloaded > 0 && !speed_initialized {
                speed_initialized = true;
                last_bytes = downloaded;
                last_stats_time = now;
            }

            let since_last_emit = now.duration_since(last_emit_time).as_millis();
            let percent_delta = (percent - last_emitted_percent).abs();

            if since_last_emit >= 500
                || percent_delta >= 1.0
                || last_emitted_percent < 0.0
            {
                last_emit_time = now;
                last_emitted_percent = percent;

                let speed = if elapsed >= EARLY_PHASE_SECS && smoothed_speed > 0.0 {
                    format_speed(smoothed_speed)
                } else {
                    String::new()
                };

                let eta = if elapsed >= EARLY_PHASE_SECS && smoothed_speed > 0.0 {
                    match total {
                        Some(t) if t > downloaded => {
                            let remaining = (t - downloaded) as f64;
                            let eta_secs = remaining / smoothed_speed;
                            format_duration(eta_secs as u64)
                        }
                        _ => String::new(),
                    }
                } else {
                    String::new()
                };

                let _ = app.emit(
                    "download-progress",
                    ProgressPayload {
                        id: id.to_string(),
                        percent,
                        speed,
                        eta,
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                    },
                );
            }
        }
    }

    drop(reader);

    let child_result = {
        let mut guard = active.lock().await;
        guard.remove(id)
    };

    let Some(mut child) = child_result else {
        return Err("Download was cancelled".to_string());
    };

    let status = child.wait().await;
    let stderr_buf = stderr_task.await.unwrap_or_default();

    match status {
        Ok(exit) if exit.success() => {
            let was_cancelled = cancelled.lock().await.contains(id);
            if was_cancelled {
                return Ok(());
            }

            let resolved_file = resolve_filename(
                yt_dlp,
                format_id,
                output_template,
                url,
            )
            .await
            .unwrap_or_else(|| format!("{}/download.mp4", output_dir));

            let _ = app.emit(
                "download-complete",
                CompletePayload {
                    id: id.to_string(),
                    path: output_dir.to_string(),
                    filename: resolved_file,
                },
            );
            Ok(())
        }
        Ok(exit) => {
            let err_text = String::from_utf8_lossy(&stderr_buf);
            let msg = if err_text.trim().is_empty() {
                format!("yt-dlp exited with code {}", exit)
            } else {
                err_text.lines().last().unwrap_or("Unknown error").to_string()
            };
            Err(msg)
        }
        Err(e) => Err(format!("Process error: {}", e)),
    }
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    state: tauri::State<'_, ProcessManager>,
    url: String,
    format_id: String,
    output_dir: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    let output_template = format!("{}/%(title)s.%(ext)s", output_dir);
    let yt_dlp = find_binary("yt-dlp");
    let ffmpeg = find_binary("ffmpeg");
    let active = state.active_downloads.clone();
    let cancelled = state.cancelled_downloads.clone();
    cancelled.lock().await.remove(&id);

    tokio::spawn(async move {
        let result = run_download_process(
            app.clone(),
            active.clone(),
            cancelled.clone(),
            &id_clone,
            &yt_dlp,
            &ffmpeg,
            &url,
            &format_id,
            &output_template,
            &output_dir,
            &[],
        )
        .await;

        let final_result = match result {
            Ok(()) => Ok(()),
            Err(msg) if is_youtube_client_error(&msg) => {
                let was_cancelled = cancelled.lock().await.contains(&id_clone);
                if was_cancelled {
                    return;
                }
                run_download_process(
                    app.clone(),
                    active.clone(),
                    cancelled.clone(),
                    &id_clone,
                    &yt_dlp,
                    &ffmpeg,
                    &url,
                    &format_id,
                    &output_template,
                    &output_dir,
                    &["--extractor-args", "youtube:player_client=web_embedded"],
                )
                .await
                .map_err(|m2| {
                    format!(
                        "{}\n\nYouTube may have changed its player clients. \
                         Updating the app (or its bundled yt-dlp) is recommended.",
                        m2
                    )
                })
            }
            Err(msg) => Err(msg),
        };

        if let Err(msg) = final_result {
            let was_cancelled = cancelled.lock().await.contains(&id_clone);
            if !was_cancelled {
                let _ = app.emit(
                    "download-error",
                    ErrorPayload {
                        id: id_clone,
                        error: msg,
                    },
                );
            }
        }
    });

    Ok(id)
}

#[tauri::command]
async fn cancel_download(
    state: tauri::State<'_, ProcessManager>,
    id: String,
) -> Result<(), String> {
    state.cancelled_downloads.lock().await.insert(id.clone());
    if let Some(mut child) = state.active_downloads.lock().await.remove(&id) {
        child
            .kill()
            .await
            .map_err(|e| format!("Failed to kill process: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn get_download_dir() -> Result<String, String> {
    if let Ok(dir) = std::env::var("YT_GRAB_OUTPUT") {
        return Ok(dir);
    }
    if let Some(home) = dirs_fallback() {
        let downloads = format!("{}/Downloads", home);
        if std::path::Path::new(&downloads).exists() {
            return Ok(downloads);
        }
    }
    Err("Could not determine downloads directory".to_string())
}

fn dirs_fallback() -> Option<String> {
    std::env::var("HOME").ok()
}

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
    } else if seconds >= 60 {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    } else {
        format!("{}s", seconds)
    }
}

async fn resolve_filename(
    yt_dlp: &str,
    format_id: &str,
    output_template: &str,
    url: &str,
) -> Option<String> {
    let mut args = vec![
        "--get-filename".to_string(),
        "-f".to_string(),
        format_id.to_string(),
        "-o".to_string(),
        output_template.to_string(),
        "--no-playlist".to_string(),
        "--no-update".to_string(),
    ];
    args.extend(js_runtime_args());
    args.push(url.to_string());

    let output = silent_command(yt_dlp)
        .args(&args)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

#[tauri::command]
async fn open_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("Failed to open file: {}", e))?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(ProcessManager {
            active_downloads: Arc::new(Mutex::new(HashMap::new())),
            cancelled_downloads: Arc::new(Mutex::new(HashSet::new())),
        })
        .invoke_handler(tauri::generate_handler![
            check_binaries,
            fetch_metadata,
            start_download,
            cancel_download,
            get_download_dir,
            open_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
