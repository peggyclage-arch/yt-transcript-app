pub mod transcript;
pub mod whisper;

use tauri::Manager;
use transcript::{CaptionTrack, Transcript};

/// Where the installer put the bundled sidecar tools (yt-dlp, ffmpeg,
/// whisper), if it kept them apart from the exe.
fn resource_dirs(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    app.path().resource_dir().ok().into_iter().collect()
}

/// Get transcript: captions first (instant), and if that fails — no network
/// to YouTube, rate-limited, no captions at all — fall back to the same
/// yt-dlp + Whisper route used for the Mega course videos: download the
/// audio and transcribe it locally. One button, it just works.
#[tauri::command]
async fn fetch_transcript(
    app: tauri::AppHandle,
    url: String,
    lang: String,
) -> Result<Transcript, String> {
    match transcript::fetch_transcript(&url, &lang).await {
        Ok(t) => Ok(t),
        Err(e) => {
            let fallback = matches!(
                &e,
                transcript::TranscriptError::FetchFailed(_)
                    | transcript::TranscriptError::RateLimited
                    | transcript::TranscriptError::NoCaptions
                    | transcript::TranscriptError::ParseFailed(_)
            );
            if !fallback {
                return Err(e.to_string());
            }
            let dirs = resource_dirs(&app);
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            whisper::transcribe_audio(&url, &lang, &dirs, &data_dir).await
        }
    }
}

#[tauri::command]
async fn list_tracks(url: String) -> Result<Vec<CaptionTrack>, String> {
    let video_id = transcript::extract_video_id(&url)
        .ok_or_else(|| "That doesn't look like a YouTube video link.".to_string())?;
    let page = transcript::list_caption_tracks(&video_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(page.tracks.into_iter().map(|(_, t)| t).collect())
}

#[tauri::command]
fn check_tools(app: tauri::AppHandle) -> whisper::ToolsStatus {
    whisper::check_tools(&resource_dirs(&app))
}

/// The app's own version, shown in the header so you can always tell which
/// build you are running.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn transcribe_audio(
    app: tauri::AppHandle,
    url: String,
    lang: String,
) -> Result<Transcript, String> {
    let dirs = resource_dirs(&app);
    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    whisper::transcribe_audio(&url, &lang, &dirs, &data_dir).await
}

/// Open the save dialog and write the transcript as a .md file.
/// Returns the saved path, or None when the user cancels.
#[tauri::command]
async fn save_transcript(
    app: tauri::AppHandle,
    content: String,
    default_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter("Markdown", &["md"])
        .set_file_name(&default_name)
        .blocking_save_file();
    let Some(path) = path else {
        return Ok(None);
    };
    let path_buf = match path.as_path() {
        Some(p) => p.to_path_buf(),
        None => return Err("Could not resolve the chosen save location.".into()),
    };
    std::fs::write(&path_buf, content).map_err(|e| e.to_string())?;
    Ok(Some(path_buf.to_string_lossy().to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            fetch_transcript,
            list_tracks,
            check_tools,
            app_version,
            transcribe_audio,
            save_transcript
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
