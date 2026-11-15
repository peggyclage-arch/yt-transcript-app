# YT Transcript

A small Tauri (Rust) Windows desktop app that does what I do when I transcribe a
YouTube video: paste a link, get the full transcript.

## What it does

1. **Captions first (fast, no download).** It fetches the video's caption tracks
   from YouTube the same way `youtube-transcript-api` does — no API key, no
   login — and picks the best track for your language (manual captions first,
   then auto-generated). This is what produced the Spencer Pawliw transcript
   (8,962 words from YouTube auto-captions).
2. **Whisper fallback.** If a video has no captions at all, it can download the
   audio with `yt-dlp` and transcribe it locally with `whisper.cpp` (tiny
   model, downloaded on first use). Both tools are optional sidecars: the app
   tells you on screen whether they are installed.
3. **Export.** Copy the transcript, or save it as a `.md` file with a header
   (title, URL, duration, source, word count) — the same format I save
   transcripts in.

## Build (Windows)

```powershell
npm install -g @tauri-apps/cli   # or: cargo install tauri-cli --version "^2"
cargo tauri build
```

The installer lands in `src-tauri/target/release/bundle/msi/`.
There is also a GitHub Actions workflow (`.github/workflows/build-windows.yml`)
that builds the Windows `.msi` on every push to `main`.

No Rust toolchain on this machine? Install it from https://rustup.rs, then run
the build above. The frontend is plain HTML/CSS/JS in `src/` — no Node build
step.

## Use

Paste a YouTube URL (watch, youtu.be, Shorts, embed, live), pick a language,
click **Get transcript**. If the video has no captions, use **Transcribe audio
(Whisper)** instead — it needs `yt-dlp` and `whisper.cpp` on your PATH, or
next to the app's `.exe`.

## Notes / limits

- YouTube may rate-limit caption fetches if you hammer it (HTTP 429) — wait a
  bit and retry. That happened once while fetching the Spencer talk; a retry
  worked.
- Auto-captions are approximate: names and a few lines can be garbled.
- All figures a speaker states (revenue, spend, CPA) are their claims — the app
  does not verify them.
