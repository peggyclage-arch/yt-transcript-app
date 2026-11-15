pub mod transcript;
pub mod whisper;

use transcript::{CaptionTrack, Transcript};

#[tauri::command]
async fn fetch_transcript(url: String, lang: String) -> Result<Transcript, String> {
    transcript::fetch_transcript(&url, &lang)
        .await
        .map_err(|e| e.to_string())
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
fn check_tools() -> whisper::ToolsStatus {
    whisper::check_tools()
}

#[tauri::command]
async fn transcribe_audio(url: String, lang: String) -> Result<Transcript, String> {
    whisper::transcribe_audio(&url, &lang).await
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
            transcribe_audio,
            save_transcript
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
