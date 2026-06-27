export interface FormatInfo {
  format_id: string;
  ext: string;
  resolution: string;
  vcodec: string;
  acodec: string;
  filesize: number | null;
  fps: number | null;
  tbr: number | null;
  has_audio: boolean;
  has_video: boolean;
  format_note: string;
}

export interface VideoMetadata {
  id: string;
  title: string;
  duration: number;
  thumbnail: string;
  webpage_url: string;
  formats: FormatInfo[];
}

export interface ProgressPayload {
  id: string;
  percent: number;
  speed: string;
  eta: string;
  downloaded_bytes: number;
  total_bytes: number | null;
}

export interface CompletePayload {
  id: string;
  path: string;
  filename: string;
}

export interface ErrorPayload {
  id: string;
  error: string;
}

export interface DownloadItem {
  id: string;
  url: string;
  title: string;
  formatLabel: string;
  outputDir: string;
  filename: string | null;
  progress: number;
  speed: string;
  eta: string;
  downloadedBytes: number;
  totalBytes: number | null;
  status: "downloading" | "completed" | "error" | "cancelled";
  error: string | null;
}
