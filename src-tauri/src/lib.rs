use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

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

    let exe_name = if cfg!(target_os = "windows") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let bundled = exe_dir.join("binaries").join(&exe_name);
            if bundled.exists() {
                return bundled.to_string_lossy().to_string();
            }

            if let Some(grandparent) = exe_dir.parent() {
                let resources = grandparent.join("Resources").join("binaries").join(&exe_name);
                if resources.exists() {
                    return resources.to_string_lossy().to_string();
                }
            }
        }
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
async fn fetch_metadata(url: String) -> Result<VideoMetadata, String> {
    let yt_dlp = find_binary("yt-dlp");

    let output = Command::new(&yt_dlp)
        .args([
            "-J",
            "--no-playlist",
            "--flat-playlist",
            "--no-check-certificate",
            &url,
        ])
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
    let app_clone = app.clone();
    let output_dir_clone = output_dir.clone();
    let output_template = format!("{}/%(title)s.%(ext)s", output_dir);
    let yt_dlp = find_binary("yt-dlp");
    let ffmpeg = find_binary("ffmpeg");

    let args: Vec<String> = vec![
        "-f".to_string(),
        format_id.clone(),
        "-o".to_string(),
        output_template.clone(),
        "--newline".to_string(),
        "--progress-template".to_string(),
        "downloaded:%(progress.downloaded_bytes)s|total:%(progress.total_bytes_estimate)s|speed:%(progress.speed)s|eta:%(progress.eta)s|pct:%(progress._percent_str)s".to_string(),
        "--ffmpeg-location".to_string(),
        ffmpeg,
        "--no-playlist".to_string(),
        "--no-mtime".to_string(),
        url.clone(),
    ];

    let mut child = Command::new(&yt_dlp)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {}", e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    state
        .active_downloads
        .lock()
        .await
        .insert(id.clone(), child);

    let active = state.active_downloads.clone();

    tokio::spawn(async move {
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

                    let _ = app_clone.emit(
                        "download-progress",
                        ProgressPayload {
                            id: id_clone.clone(),
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
            guard.remove(&id_clone)
        };

        if let Some(mut child) = child_result {
            let status = child.wait().await;
            let stderr_buf = stderr_task.await.unwrap_or_default();

            match status {
                Ok(exit) if exit.success() => {
                    let resolved_file = resolve_filename(
                        &yt_dlp,
                        &format_id,
                        &output_template,
                        &url,
                    )
                    .await
                    .unwrap_or_else(|| format!("{}/download.mp4", output_dir_clone));

                    let _ = app_clone.emit(
                        "download-complete",
                        CompletePayload {
                            id: id_clone.clone(),
                            path: output_dir_clone,
                            filename: resolved_file,
                        },
                    );
                }
                Ok(exit) => {
                    let err_text = String::from_utf8_lossy(&stderr_buf);
                    let msg = if err_text.trim().is_empty() {
                        format!("yt-dlp exited with code {}", exit)
                    } else {
                        err_text.lines().last().unwrap_or("Unknown error").to_string()
                    };
                    let _ = app_clone.emit(
                        "download-error",
                        ErrorPayload {
                            id: id_clone.clone(),
                            error: msg,
                        },
                    );
                }
                Err(e) => {
                    let _ = app_clone.emit(
                        "download-error",
                        ErrorPayload {
                            id: id_clone.clone(),
                            error: format!("Process error: {}", e),
                        },
                    );
                }
            }
        } else {
            let _ = app_clone.emit(
                "download-error",
                ErrorPayload {
                    id: id_clone.clone(),
                    error: "Download was cancelled".to_string(),
                },
            );
        }
    });

    Ok(id)
}

#[tauri::command]
async fn cancel_download(
    state: tauri::State<'_, ProcessManager>,
    id: String,
) -> Result<(), String> {
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
    let output = Command::new(yt_dlp)
        .args([
            "--get-filename",
            "-f",
            format_id,
            "-o",
            output_template,
            "--no-playlist",
            url,
        ])
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
        })
        .invoke_handler(tauri::generate_handler![
            fetch_metadata,
            start_download,
            cancel_download,
            get_download_dir,
            open_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
