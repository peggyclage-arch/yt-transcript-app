//! YouTube transcript fetching, the way `youtube-transcript-api` does it.
//!
//! No API key of our own, no login: ask YouTube's InnerTube player API
//! (ANDROID client, the same public key `youtube-transcript-api` uses) for
//! the video's caption track list, then download the chosen track as JSON3.
//! Manual captions are preferred over auto-generated ones (`kind = "asr"`),
//! and the requested language is preferred over English.
//!
//! Why InnerTube and not the watch page: the caption URLs in a scraped watch
//! page are unsigned, and YouTube answers those timedtext requests with an
//! empty 200. The player API returns signed URLs (expire + signature) that
//! actually serve the track. The track URL already carries `fmt=srv3`, so the
//! format has to be *replaced* with `fmt=json3`, not appended — a duplicate
//! fmt parameter makes YouTube honour the first one and return XML.

use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const INNERTUBE_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const INNERTUBE_URL: &str = "https://www.youtube.com/youtubei/v1/player";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub video_id: String,
    pub title: String,
    pub language_code: String,
    pub language_name: String,
    pub is_auto_generated: bool,
    pub segments: Vec<Segment>,
    pub full_text: String,
    pub word_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptionTrack {
    pub language_code: String,
    pub language_name: String,
    pub is_auto_generated: bool,
}

/// Everything that can go wrong, in words a user can act on.
#[derive(Debug)]
pub enum TranscriptError {
    InvalidUrl,
    FetchFailed(String),
    RateLimited,
    VideoUnavailable,
    NoCaptions,
    ParseFailed(String),
}

impl std::fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptError::InvalidUrl => {
                write!(f, "That doesn't look like a YouTube video link.")
            }
            TranscriptError::FetchFailed(e) => write!(f, "Could not reach YouTube: {e}"),
            TranscriptError::RateLimited => write!(
                f,
                "YouTube rate-limited the request (HTTP 429). Wait a few minutes and try again."
            ),
            TranscriptError::VideoUnavailable => {
                write!(f, "That video can't be opened (removed, private, or age-restricted).")
            }
            TranscriptError::NoCaptions => write!(
                f,
                "This video has no captions in any language. Use \"Transcribe audio (Whisper)\" instead."
            ),
            TranscriptError::ParseFailed(e) => write!(
                f,
                "YouTube changed its format and I couldn't read it: {e}"
            ),
        }
    }
}

impl std::error::Error for TranscriptError {}

/// Accepts watch, youtu.be, Shorts, embed, live and music links.
pub fn extract_video_id(url: &str) -> Option<String> {
    let url = url.trim();
    // watch?v=VIDEO_ID (also handles &v=, ?v=, /watch/...?v=)
    if let Some(pos) = url.find("v=") {
        let rest = &url[pos + 2..];
        let end = rest
            .find(|c| c == '&' || c == '#' || c == '?')
            .unwrap_or(rest.len());
        let id = &rest[..end];
        if id.len() == 11 {
            return Some(id.to_string());
        }
    }
    // youtu.be/VIDEO_ID, /shorts/VIDEO_ID, /embed/VIDEO_ID, /live/VIDEO_ID, /v/VIDEO_ID
    for marker in ["youtu.be/", "/shorts/", "/embed/", "/live/", "/v/"] {
        if let Some(pos) = url.find(marker) {
            let rest = &url[pos + marker.len()..];
            let end = rest
                .find(|c| c == '&' || c == '#' || c == '?' || c == '/')
                .unwrap_or(rest.len());
            let id = &rest[..end];
            if id.len() == 11 {
                return Some(id.to_string());
            }
        }
    }
    // Bare 11-char video id pasted on its own.
    if url.len() == 11 && url.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Some(url.to_string());
    }
    None
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client builds")
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[derive(Debug, Deserialize)]
struct Json3Seg {
    utf8: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Json3Event {
    #[serde(rename = "tStartMs")]
    t_start_ms: Option<i64>,
    segs: Option<Vec<Json3Seg>>,
}

#[derive(Debug, Deserialize)]
struct Json3 {
    events: Option<Vec<Json3Event>>,
}

pub struct FetchedPage {
    pub title: String,
    pub tracks: Vec<(String, CaptionTrack)>, // (base_url, track)
}

/// Ask the InnerTube player API for a video's caption tracks.
pub async fn list_caption_tracks(video_id: &str) -> Result<FetchedPage, TranscriptError> {
    let url = format!("{INNERTUBE_URL}?key={INNERTUBE_KEY}&prettyPrint=false");
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "ANDROID",
                "clientVersion": "20.10.38",
                "hl": "en",
                "gl": "US",
            }
        },
        "videoId": video_id,
    });
    let resp = http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| TranscriptError::FetchFailed(e.to_string()))?;

    if resp.status().as_u16() == 429 {
        return Err(TranscriptError::RateLimited);
    }
    if !resp.status().is_success() {
        return Err(TranscriptError::VideoUnavailable);
    }
    let player: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TranscriptError::ParseFailed(e.to_string()))?;

    let playability = player
        .pointer("/playabilityStatus/status")
        .and_then(|v| v.as_str())
        .unwrap_or("OK");
    if playability != "OK" {
        return Err(TranscriptError::VideoUnavailable);
    }
    let title = player
        .pointer("/videoDetails/title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut tracks = Vec::new();
    if let Some(list) = player
        .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
        .and_then(|v| v.as_array())
    {
        for t in list {
            let base_url = match t.get("baseUrl").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(),
                None => continue,
            };
            let language_code = t
                .get("languageCode")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let language_name = t
                .pointer("/name/simpleText")
                .and_then(|v| v.as_str())
                .unwrap_or(&language_code)
                .to_string();
            let is_auto_generated = t.get("kind").and_then(|v| v.as_str()) == Some("asr");
            tracks.push((
                base_url,
                CaptionTrack {
                    language_code,
                    language_name,
                    is_auto_generated,
                },
            ));
        }
    }
    Ok(FetchedPage { title, tracks })
}

fn pick_track<'a>(
    tracks: &'a [(String, CaptionTrack)],
    lang: &str,
) -> Option<&'a (String, CaptionTrack)> {
    let want = lang.trim().to_lowercase();
    // 1. manual captions in the requested language
    // 2. auto captions in the requested language
    // 3. manual captions in English, 4. auto in English, 5. any manual, 6. any auto
    let prefs: Vec<(&str, bool)> = if want.is_empty() || want == "auto" {
        vec![("en", false), ("en", true)]
    } else {
        vec![
            (want.as_str(), false),
            (want.as_str(), true),
            ("en", false),
            ("en", true),
        ]
    };
    for (code, auto) in prefs {
        if let Some(t) = tracks
            .iter()
            .find(|(_, tr)| tr.language_code == code && tr.is_auto_generated == auto)
        {
            return Some(t);
        }
    }
    tracks
        .iter()
        .find(|(_, tr)| !tr.is_auto_generated)
        .or_else(|| tracks.first())
}

/// The track URL already carries `fmt=srv3`. Replace it with `fmt=json3` —
/// appending a second fmt parameter makes YouTube honour the first and
/// return XML instead of JSON.
fn with_json3_format(url: &str) -> String {
    if let Some(pos) = url.find("fmt=") {
        let after = &url[pos + 4..];
        let end = after.find('&').map(|i| pos + 4 + i).unwrap_or(url.len());
        format!("{}fmt=json3{}", &url[..pos], &url[end..])
    } else if url.contains('?') {
        format!("{url}&fmt=json3")
    } else {
        format!("{url}?fmt=json3")
    }
}

/// Fetch the full transcript for a video URL. `lang` is a language code
/// ("en", "fr", "ar", ...) or "auto" for English-first.
pub async fn fetch_transcript(url: &str, lang: &str) -> Result<Transcript, TranscriptError> {
    let video_id = extract_video_id(url).ok_or(TranscriptError::InvalidUrl)?;
    let page = list_caption_tracks(&video_id).await?;
    if page.tracks.is_empty() {
        return Err(TranscriptError::NoCaptions);
    }
    let (base_url, track) = pick_track(&page.tracks, lang).ok_or(TranscriptError::NoCaptions)?;
    let base_url = base_url.clone();
    let track = track.clone();
    let track_url = with_json3_format(&base_url);
    let resp = http_client()
        .get(&track_url)
        .send()
        .await
        .map_err(|e| TranscriptError::FetchFailed(e.to_string()))?;
    if resp.status().as_u16() == 429 {
        return Err(TranscriptError::RateLimited);
    }
    let body = resp
        .text()
        .await
        .map_err(|e| TranscriptError::FetchFailed(e.to_string()))?;
    let json3: Json3 =
        serde_json::from_str(&body).map_err(|e| TranscriptError::ParseFailed(e.to_string()))?;

    let mut segments = Vec::new();
    for ev in json3.events.unwrap_or_default() {
        let Some(segs) = ev.segs else { continue };
        let text: String = segs
            .into_iter()
            .filter_map(|s| s.utf8)
            .collect::<Vec<_>>()
            .join("")
            .replace('\n', " ");
        let text = decode_entities(&text);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start_secs: ev.t_start_ms.unwrap_or(0) as f64 / 1000.0,
            text,
        });
    }
    if segments.is_empty() {
        return Err(TranscriptError::NoCaptions);
    }
    let full_text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let word_count = full_text.split_whitespace().count();
    Ok(Transcript {
        video_id,
        title: page.title,
        language_code: track.language_code,
        language_name: track.language_name,
        is_auto_generated: track.is_auto_generated,
        segments,
        full_text,
        word_count,
    })
}

/// Render the transcript exactly like the `.md` files I save: header + timestamped lines.
pub fn render_markdown(t: &Transcript, source_url: &str, source_label: &str) -> String {
    let mut out = String::new();
    let title = if t.title.is_empty() {
        "YouTube transcript"
    } else {
        &t.title
    };
    out.push_str(&format!("# {title}\n"));
    out.push_str(&format!("# {source_url}\n"));
    out.push_str(&format!(
        "# source: {source_label} ({}) | {} words\n\n",
        if t.is_auto_generated {
            "auto-generated captions"
        } else {
            "manual captions"
        },
        t.word_count
    ));
    for s in &t.segments {
        let total = s.start_secs as u64;
        out.push_str(&format!(
            "[{:02}:{:02}] {}\n",
            total / 60,
            total % 60,
            s.text
        ));
    }
    out
}
