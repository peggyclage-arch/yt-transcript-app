//! Whisper fallback: download the audio with yt-dlp, transcribe it locally
//! with whisper.cpp. Used only when a video has no captions at all — the
//! same fallback I use (Whisper where captions are unavailable).

use crate::transcript::{Segment, Transcript};
use serde::Deserialize;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub yt_dlp: bool,
    pub ffmpeg: bool,
    pub whisper: bool,
}

fn find_binary(name: &str, extra_dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    // Where to look, in order. The MSI installer puts bundled tools in a
    // `resources` subfolder of the install dir (NOT next to the exe), so
    // every base dir is also searched with `/resources` appended — that was
    // the v0.1.1 bug: whisper.exe sat in <install>\resources while the app
    // only looked next to the exe, so it reported whisper.cpp missing even
    // though the installer shipped it.
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    // 1. Next to the app's .exe (portable sidecars), and its resources subfolder.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("resources"));
        }
    }
    // 2. The app's resource dir (where the installer puts bundled tools),
    //    and its resources subfolder.
    for d in extra_dirs {
        dirs.push(d.clone());
        dirs.push(d.join("resources"));
    }
    for dir in &dirs {
        for cand in [format!("{name}.exe"), name.to_string()] {
            let p = dir.join(&cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // 3. On PATH.
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for cand in [format!("{name}.exe"), name.to_string()] {
                let p = dir.join(&cand);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn check_tools(extra_dirs: &[std::path::PathBuf]) -> ToolsStatus {
    ToolsStatus {
        yt_dlp: find_binary("yt-dlp", extra_dirs).is_some(),
        ffmpeg: find_binary("ffmpeg", extra_dirs).is_some(),
        whisper: find_binary("whisper", extra_dirs).is_some(),
    }
}

fn parse_whisper_ts(s: &str) -> f64 {
    // "HH:MM:SS,mmm"
    let parts: Vec<&str> = s.split([':', ',']).collect();
    if parts.len() == 4 {
        let h: f64 = parts[0].parse().unwrap_or(0.0);
        let m: f64 = parts[1].parse().unwrap_or(0.0);
        let sec: f64 = parts[2].parse().unwrap_or(0.0);
        let ms: f64 = parts[3].parse().unwrap_or(0.0);
        h * 3600.0 + m * 60.0 + sec + ms / 1000.0
    } else {
        0.0
    }
}

#[derive(Debug, Deserialize)]
struct WhisperJson {
    transcription: Option<Vec<WhisperEntry>>,
}

#[derive(Debug, Deserialize)]
struct WhisperEntry {
    timestamps: Option<WhisperTimestamps>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhisperTimestamps {
    from: Option<String>,
}

const TINY_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";

/// The model lives in the app's data dir (writable by a normal user), not
/// next to the exe — an MSI install lands in Program Files, which a
/// standard user cannot write to.
fn model_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("models").join("ggml-tiny.bin")
}

async fn ensure_model(data_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let path = model_path(data_dir);
    if path.is_file() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The model download used to have no timeout at all: if the connection
    // to HuggingFace stalled, the app sat on the spinner forever (Boss hit
    // this on v0.1.2). 120s is generous for a 75 MB file; a stall now fails
    // with a clear error instead of hanging.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Could not download the Whisper tiny model: {e}"))?;
    let bytes = client
        .get(TINY_MODEL_URL)
        .send()
        .await
        .map_err(|e| format!("Could not download the Whisper tiny model: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Could not download the Whisper tiny model: {e}"))?;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Download audio with yt-dlp and transcribe it with whisper.cpp.
/// Returns a Transcript shaped exactly like the captions path.
/// `extra_dirs` are extra places to look for the sidecar tools (the app's
/// resource dir); `data_dir` is where the Whisper model is kept.
pub async fn transcribe_audio(
    url: &str,
    lang: &str,
    extra_dirs: &[std::path::PathBuf],
    data_dir: &std::path::Path,
) -> Result<Transcript, String> {
    let video_id = crate::transcript::extract_video_id(url)
        .ok_or_else(|| "That doesn't look like a YouTube video link.".to_string())?;
    let ytdlp = find_binary("yt-dlp", extra_dirs)
        .ok_or_else(|| "yt-dlp is not installed. Put yt-dlp.exe next to this app or on your PATH.".to_string())?;
    let whisper = find_binary("whisper", extra_dirs)
        .ok_or_else(|| "whisper.cpp is not installed. Put whisper.exe next to this app or on your PATH.".to_string())?;
    if find_binary("ffmpeg", extra_dirs).is_none() {
        return Err("ffmpeg is not installed — yt-dlp needs it to extract audio. Put ffmpeg.exe next to this app or on your PATH.".into());
    }
    let model = ensure_model(data_dir).await?;
    let title = crate::transcript::list_caption_tracks(&video_id)
        .await
        .map(|p| p.title)
        .unwrap_or_default();

    let workdir = std::env::temp_dir().join("yt-transcript").join(&video_id);
    std::fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    let url = url.to_string();
    let lang = lang.to_string();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<Transcript, String> {
        let audio_out = workdir.join("audio.%(ext)s");
        let dl = std::process::Command::new(&ytdlp)
            .args([
                "-f", "bestaudio",
                "--extract-audio",
                "--audio-format", "wav",
                "--audio-quality", "0",
                "-o", audio_out.to_str().unwrap_or("audio.%(ext)s"),
                "--no-playlist",
                &url,
            ])
            .output()
            .map_err(|e| format!("Could not run yt-dlp: {e}"))?;
        if !dl.status.success() {
            let err = String::from_utf8_lossy(&dl.stderr);
            return Err(format!("yt-dlp failed to download the audio: {}", tail(&err, 500)));
        }
        let audio = workdir.join("audio.wav");
        if !audio.is_file() {
            return Err("yt-dlp finished but no audio.wav was produced.".into());
        }
        let out_base = workdir.join("out");
        let mut cmd = std::process::Command::new(&whisper);
        cmd.args(["-m", model.to_str().unwrap_or(""), "-f", audio.to_str().unwrap_or(""), "-oj", "-of", out_base.to_str().unwrap_or("out")]);
        if lang != "auto" && !lang.is_empty() {
            cmd.args(["-l", &lang]);
        }
        let run = cmd.output().map_err(|e| format!("Could not run whisper: {e}"))?;
        if !run.status.success() {
            let err = String::from_utf8_lossy(&run.stderr);
            return Err(format!("whisper failed: {}", tail(&err, 500)));
        }
        let json_path = workdir.join("out.json");
        let raw = std::fs::read_to_string(&json_path)
            .map_err(|e| format!("whisper finished but no out.json was produced: {e}"))?;
        let parsed: WhisperJson =
            serde_json::from_str(&raw).map_err(|e| format!("Could not read whisper's output: {e}"))?;
        let mut segments = Vec::new();
        for e in parsed.transcription.unwrap_or_default() {
            let text = e.text.unwrap_or_default();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                continue;
            }
            let start = e
                .timestamps
                .and_then(|t| t.from)
                .map(|s| parse_whisper_ts(&s))
                .unwrap_or(0.0);
            segments.push(Segment { start_secs: start, text });
        }
        if segments.is_empty() {
            return Err("whisper produced no text for this video.".into());
        }
        let full_text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let word_count = full_text.split_whitespace().count();
        Ok(Transcript {
            video_id,
            title,
            language_code: if lang == "auto" { "auto".into() } else { lang },
            language_name: "Whisper transcription".into(),
            is_auto_generated: true,
            segments,
            full_text,
            word_count,
        })
    })
    .await
    .map_err(|e| format!("Transcription task failed: {e}"))??;
    Ok(result)
}

fn tail(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - n..])
    }
}
