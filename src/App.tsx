import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useDownloadStore } from "./store/downloads";
import type {
  VideoMetadata,
  FormatInfo,
  ProgressPayload,
  CompletePayload,
  ErrorPayload,
} from "./types";
import "./App.css";

interface ComboEntry {
  resolution: string;
  resolutionSort: number;
  videoId: string;
  audioId: string;
  combinedId: string;
  container: "webm" | "mp4";
  videoCodec: string;
  audioCodec: string;
  videoExt: string;
  audioExt: string;
  videoSize: number | null;
  audioSize: number | null;
  fps: number | null;
}

type TabMode = "combos" | "audio" | "video";

function formatSize(bytes: number | null): string {
  if (bytes === null || bytes === undefined) return "?";
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(0)} KB`;
  return `${bytes} B`;
}

function totalSize(a: number | null, b: number | null): string {
  if (a === null && b === null) return "?";
  return formatSize((a ?? 0) + (b ?? 0));
}

function formatTime(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function shortCodec(raw: string): string {
  const parts = raw.split(".");
  return parts[0] || raw;
}

function resolutionSortKey(res: string): number {
  if (res === "?") return 0;
  const parts = res.split("x");
  return parseInt(parts[0]) || 0;
}

function formatLabelFromId(metadata: VideoMetadata, formatId: string): string {
  if (formatId.includes("+")) {
    const [vid, aud] = formatId.split("+");
    const vfmt = metadata.formats.find((f) => f.format_id === vid);
    const afmt = metadata.formats.find((f) => f.format_id === aud);
    const vlabel = vfmt?.resolution ?? vid;
    const alabel = afmt?.format_note ?? aud;
    return `${vlabel} + ${alabel}`;
  }
  const fmt = metadata.formats.find((f) => f.format_id === formatId);
  if (!fmt) return formatId;
  if (fmt.has_video && fmt.resolution !== "?") return fmt.resolution;
  return fmt.format_note || fmt.format_id;
}

function buildCombos(formats: FormatInfo[]): ComboEntry[] {
  const videoFormats = formats.filter(
    (f) => f.has_video && !f.has_audio && f.resolution !== "?" && !f.format_id.includes("m3u8")
  );
  const audioFormats = formats.filter(
    (f) => f.has_audio && !f.has_video && !f.format_id.includes("m3u8")
  );

  const audioByExt: Record<string, FormatInfo[]> = {};
  for (const a of audioFormats) {
    if (!audioByExt[a.ext]) audioByExt[a.ext] = [];
    audioByExt[a.ext].push(a);
  }
  for (const ext of Object.keys(audioByExt)) {
    audioByExt[ext].sort((a, b) => (b.tbr ?? 0) - (a.tbr ?? 0));
  }

  const comboMap = new Map<string, ComboEntry>();

  for (const v of videoFormats) {
    const container = v.ext === "mp4" ? "mp4" : "webm";
    const audioExt = container === "mp4" ? "m4a" : "webm";
    const bestAudio = audioByExt[audioExt]?.[0];
    if (!bestAudio) continue;

    const key = `${container}:${v.resolution}`;
    const existing = comboMap.get(key);
    if (existing && (v.tbr ?? 0) <= (existing.videoSize ?? 0)) continue;

    comboMap.set(key, {
      resolution: v.resolution,
      resolutionSort: resolutionSortKey(v.resolution),
      videoId: v.format_id,
      audioId: bestAudio.format_id,
      combinedId: `${v.format_id}+${bestAudio.format_id}`,
      container,
      videoCodec: shortCodec(v.vcodec),
      audioCodec: shortCodec(bestAudio.acodec),
      videoExt: v.ext,
      audioExt: bestAudio.ext,
      videoSize: v.filesize,
      audioSize: bestAudio.filesize,
      fps: v.fps,
    });
  }

  const combos = [...comboMap.values()];
  combos.sort((a, b) => b.resolutionSort - a.resolutionSort);
  return combos;
}

function App() {
  const [url, setUrl] = useState("");
  const [metadata, setMetadata] = useState<VideoMetadata | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedFormat, setSelectedFormat] = useState<string | null>(null);
  const [outputDir, setOutputDir] = useState<string>("");
  const [tabMode, setTabMode] = useState<TabMode>("combos");
  const [autoFetch, setAutoFetch] = useState(true);
  const inputRef = useRef<HTMLInputElement>(null);

  const store = useDownloadStore();
  const downloads = Object.values(store.items);

  useEffect(() => {
    invoke<string>("get_download_dir")
      .then(setOutputDir)
      .catch(() => setOutputDir("~/Downloads"));

    const unlistenProgress = listen<ProgressPayload>(
      "download-progress",
      (event) => {
        store.updateProgress(
          event.payload.id,
          event.payload.percent,
          event.payload.speed,
          event.payload.eta,
          event.payload.downloaded_bytes,
          event.payload.total_bytes,
        );
      },
    );

    const unlistenComplete = listen<CompletePayload>(
      "download-complete",
      (event) => {
        store.markComplete(event.payload.id, event.payload.filename);
      },
    );

    const unlistenError = listen<ErrorPayload>(
      "download-error",
      (event) => {
        store.markError(event.payload.id, event.payload.error);
      },
    );

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  const fetchMetadata = useCallback(async () => {
    if (!url.trim()) return;
    setLoading(true);
    setError(null);
    setMetadata(null);
    setSelectedFormat(null);
    setTabMode("combos");
    try {
      const data = await invoke<VideoMetadata>("fetch_metadata", { url: url.trim() });
      setMetadata(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [url]);

  const handleInputFocus = useCallback(async () => {
    if (!autoFetch) return;
    try {
      const text = await readText();
      if (text && (text.includes("youtube.com/watch") || text.includes("youtu.be/"))) {
        const clean = text.trim().split(/[\s\r\n]+/)[0];
        setUrl(clean);
        setLoading(true);
        setError(null);
        setMetadata(null);
        setSelectedFormat(null);
        setTabMode("combos");
        try {
          const data = await invoke<VideoMetadata>("fetch_metadata", { url: clean });
          setMetadata(data);
        } catch (e) {
          setError(String(e));
        } finally {
          setLoading(false);
        }
      }
    } catch {
      // clipboard read failed — ignore silently
    }
  }, [autoFetch]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") fetchMetadata();
  };

  const selectOutputDir = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Select output folder" });
    if (dir) setOutputDir(dir as string);
  };

  const startDownload = async (formatId?: string) => {
    if (!metadata) return;
    const fmt = formatId ?? selectedFormat;
    if (!fmt) return;

    const displayLabel = formatLabelFromId(metadata, fmt);

    const id = await invoke<string>("start_download", {
      url: metadata.webpage_url,
      formatId: fmt,
      outputDir,
    });

    store.addItem({
      id,
      url: metadata.webpage_url,
      title: metadata.title,
      formatLabel: displayLabel,
      outputDir,
      filename: null,
      progress: 0,
      speed: "",
      eta: "",
      downloadedBytes: 0,
      totalBytes: null,
      status: "downloading",
      error: null,
    });
  };

  const cancelDownload = async (id: string) => {
    try {
      await invoke("cancel_download", { id });
      store.markCancelled(id);
    } catch (e) {
      console.error("Cancel failed:", e);
    }
  };

  const handleOpenFile = async (filename: string) => {
    try {
      await invoke("open_file", { path: filename });
    } catch (e) {
      console.error("Open file failed:", e);
    }
  };

  const videoOnlyFormats = (metadata?.formats ?? []).filter(
    (f) => f.has_video && !f.has_audio && f.format_note !== "storyboard" && !f.format_id.includes("m3u8")
  );

  const audioOnlyFormats = (metadata?.formats ?? []).filter(
    (f) => f.has_audio && !f.has_video && !f.format_id.includes("m3u8")
  );

  const combos = metadata ? buildCombos(metadata.formats) : [];

  return (
    <div className="app">
      <header className="header">
        <h1>YT Grab</h1>
        <span className="subtitle">YouTube Video Downloader</span>
      </header>

      <div className="main-layout">
        <div className="panel panel-left">
          <div className="section">
            <div className="input-row">
              <button
                onClick={() => setAutoFetch(!autoFetch)}
                className={`sniff-toggle ${autoFetch ? "active" : ""}`}
                title={autoFetch ? "Auto-fetch: ON \u2014 click to disable" : "Auto-fetch: OFF \u2014 click to enable"}
              >
                <span className="sniff-icon">🐽</span>
                <span className="sniff-label">{autoFetch ? "ON" : "OFF"}</span>
              </button>
              <input
                ref={inputRef}
                type="text"
                placeholder={autoFetch ? "Click here to auto-paste from clipboard..." : "Paste YouTube URL here..."}
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={handleKeyDown}
                onFocus={handleInputFocus}
                className="url-input"
              />
              <button
                onClick={fetchMetadata}
                disabled={loading || !url.trim()}
                className="btn btn-primary"
              >
                {loading ? "Loading..." : "Fetch"}
              </button>
            </div>
            {error && <div className="error-box">{error}</div>}
          </div>

          {metadata && (
            <div className="section metadata-card">
              {metadata.thumbnail && (
                <img
                  src={metadata.thumbnail}
                  alt={metadata.title}
                  className="thumbnail"
                />
              )}
              <div className="meta-info">
                <h2 className="video-title">{metadata.title}</h2>
                <p className="video-meta">
                  Duration: {formatTime(metadata.duration)} &middot; ID: {metadata.id}
                </p>
              </div>
            </div>
          )}

          {metadata && combos.length > 0 && (() => {
            const topMp4 = combos.find((c) => c.container === "mp4") ?? combos[0];
            return (
              <div className="section quick-download">
                <div className="quick-dl-row">
                  <div className="quick-dl-info">
                    <span className="quick-dl-label">Best MP4</span>
                    <span className="quick-dl-resolution">{topMp4.resolution}</span>
                    <span className="quick-dl-codec">
                      {topMp4.videoCodec} + {topMp4.audioCodec}
                    </span>
                    <span className="quick-dl-size">{totalSize(topMp4.videoSize, topMp4.audioSize)}</span>
                  </div>
                  <button
                    onClick={() => startDownload(topMp4.combinedId)}
                    disabled={!outputDir}
                    className="btn btn-primary quick-dl-btn"
                  >
                    Download {topMp4.resolution} MP4
                  </button>
                </div>
              </div>
            );
          })()}

          {metadata && metadata.formats.length > 0 && (
            <div className="section">
              <div className="section-header">
                <h3>Format Selection</h3>
                <div className="tab-bar">
                  <button
                    className={`tab ${tabMode === "combos" ? "active" : ""}`}
                    onClick={() => setTabMode("combos")}
                  >
                    Combos
                  </button>
                  <button
                    className={`tab ${tabMode === "audio" ? "active" : ""}`}
                    onClick={() => setTabMode("audio")}
                  >
                    Audio
                  </button>
                  <button
                    className={`tab ${tabMode === "video" ? "active" : ""}`}
                    onClick={() => setTabMode("video")}
                  >
                    Video
                  </button>
                </div>
              </div>

              {tabMode === "combos" && (
                <>
                  <div className="format-table-wrap">
                    <table className="format-table">
                      <thead>
                        <tr>
                          <th>Quality</th>
                          <th>Container</th>
                          <th>Video + Audio Codec</th>
                          <th>Total Size</th>
                          <th>FPS</th>
                          <th></th>
                        </tr>
                      </thead>
                      <tbody>
                        {combos.map((c) => (
                          <tr
                            key={c.combinedId}
                            className={selectedFormat === c.combinedId ? "selected" : ""}
                            onClick={() => setSelectedFormat(c.combinedId)}
                          >
                            <td className="quality-cell">
                              <span className="quality-badge">{c.resolution}</span>
                              <span className="tag tag-green">A+V</span>
                              {c.container === "mp4" && (
                                <span className="tag tag-recommend">Recommended</span>
                              )}
                            </td>
                            <td className="mono">{c.container}</td>
                            <td className="mono">
                              {c.videoCodec} + {c.audioCodec}
                            </td>
                            <td>{totalSize(c.videoSize, c.audioSize)}</td>
                            <td>{c.fps ? `${c.fps}` : "-"}</td>
                            <td>
                              <button
                                className={`btn btn-sm ${selectedFormat === c.combinedId ? "btn-selected" : "btn-outline"}`}
                                onClick={(e) => { e.stopPropagation(); setSelectedFormat(c.combinedId); }}
                              >
                                {selectedFormat === c.combinedId ? "Selected" : "Select"}
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {combos.length === 0 && (
                    <div className="empty-tab">No combos available for this video.</div>
                  )}
                </>
              )}

              {tabMode === "audio" && (
                <div className="format-table-wrap">
                  <table className="format-table">
                    <thead>
                      <tr>
                        <th>Quality</th>
                        <th>Codec</th>
                        <th>Container</th>
                        <th>Size</th>
                        <th></th>
                      </tr>
                    </thead>
                    <tbody>
                      {audioOnlyFormats
                        .sort((a, b) => (b.tbr ?? 0) - (a.tbr ?? 0))
                        .map((f) => (
                          <tr
                            key={f.format_id}
                            className={selectedFormat === f.format_id ? "selected" : ""}
                            onClick={() => setSelectedFormat(f.format_id)}
                          >
                            <td className="quality-cell">
                              <span className="quality-badge">{f.format_note || "audio"}</span>
                              <span className="tag tag-blue">audio</span>
                            </td>
                            <td className="mono">{shortCodec(f.acodec)}</td>
                            <td>{f.ext}</td>
                            <td>{formatSize(f.filesize)}</td>
                            <td>
                              <button
                                className={`btn btn-sm ${selectedFormat === f.format_id ? "btn-selected" : "btn-outline"}`}
                                onClick={(e) => { e.stopPropagation(); setSelectedFormat(f.format_id); }}
                              >
                                {selectedFormat === f.format_id ? "Selected" : "Select"}
                              </button>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )}

              {tabMode === "video" && (
                <div className="format-table-wrap">
                  <table className="format-table">
                    <thead>
                      <tr>
                        <th>Quality</th>
                        <th>Codec</th>
                        <th>Container</th>
                        <th>Size</th>
                        <th>FPS</th>
                        <th></th>
                      </tr>
                    </thead>
                    <tbody>
                      {videoOnlyFormats
                        .sort((a, b) => resolutionSortKey(b.resolution) - resolutionSortKey(a.resolution))
                        .map((f) => (
                          <tr
                            key={f.format_id}
                            className={selectedFormat === f.format_id ? "selected" : ""}
                            onClick={() => {
                              const matchingCombo = combos.find((c) => c.videoId === f.format_id);
                              setSelectedFormat(matchingCombo ? matchingCombo.combinedId : f.format_id);
                            }}
                          >
                            <td className="quality-cell">
                              <span className="quality-badge">{f.resolution}</span>
                              <span className="tag tag-yellow">video</span>
                            </td>
                            <td className="mono">{shortCodec(f.vcodec)}</td>
                            <td>{f.ext}</td>
                            <td>{formatSize(f.filesize)}</td>
                            <td>{f.fps ? `${f.fps}` : "-"}</td>
                            <td>
                              <button
                                className={`btn btn-sm ${selectedFormat === f.format_id || selectedFormat === combos.find((c) => c.videoId === f.format_id)?.combinedId ? "btn-selected" : "btn-outline"}`}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  const c = combos.find((co) => co.videoId === f.format_id);
                                  setSelectedFormat(c ? c.combinedId : f.format_id);
                                }}
                              >
                                {selectedFormat === f.format_id || selectedFormat === combos.find((c) => c.videoId === f.format_id)?.combinedId ? "Selected" : "Select"}
                              </button>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )}

              <div className="download-bar">
                <div className="output-row">
                  <input
                    type="text"
                    value={outputDir}
                    onChange={(e) => setOutputDir(e.target.value)}
                    className="path-input"
                    placeholder="Output directory..."
                  />
                  <button onClick={selectOutputDir} className="btn btn-outline btn-sm">
                    Browse
                  </button>
                </div>
                <button
                  onClick={() => startDownload()}
                  disabled={!selectedFormat}
                  className="btn btn-primary btn-lg"
                >
                  Download{selectedFormat ? ` (${formatLabelFromId(metadata, selectedFormat)})` : ""}
                </button>
              </div>
            </div>
          )}
        </div>

        <div className="panel panel-right">
          <div className="section">
            <div className="section-header">
              <h3>Downloads</h3>
              {store.items && Object.keys(store.items).length > 0 && (
                <button onClick={() => store.clearCompleted()} className="btn btn-sm btn-outline">
                  Clear done
                </button>
              )}
            </div>
            {downloads.length === 0 ? (
              <div className="empty-state">
                <p>No downloads yet. Paste a URL and select a format to begin.</p>
              </div>
            ) : (
              <div className="download-list">
                {downloads.map((item) => (
                  <div key={item.id} className={`download-item status-${item.status}`}>
                    <div className="download-badge-row">
                      <span className="download-quality-badge">{item.formatLabel}</span>
                    </div>
                    <div className="download-title-row">
                      <span className="download-title" title={item.title}>
                        {item.title}
                      </span>
                    </div>
                    {item.status === "downloading" && (
                      <div className="progress-wrap">
                        <div className="progress-bar">
                          <div
                            className="progress-fill"
                            style={{ width: `${item.progress}%` }}
                          />
                        </div>
                        <div className="progress-stats">
                          <span className="progress-pct">
                            {item.progress > 0 ? `${item.progress.toFixed(1)}%` : "Preparing..."}
                          </span>
                          {item.downloadedBytes > 0 && (
                            <span className="progress-size">
                              {formatSize(item.downloadedBytes)}
                              {item.totalBytes ? ` / ${formatSize(item.totalBytes)}` : ""}
                            </span>
                          )}
                          {item.speed && <span>{item.speed}</span>}
                          {item.eta && <span>ETA: {item.eta}</span>}
                        </div>
                        <button
                          onClick={() => cancelDownload(item.id)}
                          className="btn btn-sm btn-danger"
                        >
                          Cancel
                        </button>
                      </div>
                    )}
                    {item.status === "completed" && (
                      <div className="completed-row">
                        <div className="status-row">
                          <span className="status-badge status-ok">Completed</span>
                          <span className="path-hint" title={item.filename ?? item.outputDir}>
                            {item.filename ?? item.outputDir}
                          </span>
                        </div>
                        <div className="completed-actions">
                          {item.filename && (
                            <button
                              onClick={() => handleOpenFile(item.filename!)}
                              className="btn btn-sm btn-primary"
                            >
                              Open File
                            </button>
                          )}
                          <button
                            onClick={() => store.removeItem(item.id)}
                            className="btn btn-sm btn-outline"
                          >
                            Dismiss
                          </button>
                        </div>
                      </div>
                    )}
                    {item.status === "error" && (
                      <div className="status-row">
                        <span className="status-badge status-err">Failed</span>
                        <span className="error-msg" title={item.error ?? ""}>
                          {item.error ?? "Unknown error"}
                        </span>
                        <button
                          onClick={() => store.removeItem(item.id)}
                          className="btn btn-sm btn-outline"
                        >
                          Dismiss
                        </button>
                      </div>
                    )}
                    {item.status === "cancelled" && (
                      <div className="status-row">
                        <span className="status-badge status-cancel">Cancelled</span>
                        <button
                          onClick={() => store.removeItem(item.id)}
                          className="btn btn-sm btn-outline"
                        >
                          Dismiss
                        </button>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
