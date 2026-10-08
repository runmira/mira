//! `browser_record_start` / `browser_record_stop`: a recording of the
//! browser the user watches, as an animated GIF.
//!
//! The live view already streams the page as JPEG frames (CDP screencast);
//! recording subscribes to that stream. Frames arrive only when the page
//! repaints, so each one is held until the next: a still page records as a
//! still, at no cost. Sampled to about 5 frames a second, capped at three
//! minutes, and scaled to 960px wide so a GIF stays shareable.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;

use crate::protocol::ServerMsg;
use crate::slot::SessionSlot;
use crate::state::AppState;

const MIN_GAP: Duration = Duration::from_millis(200);
const MAX_LEN: Duration = Duration::from_secs(180);
const MAX_WIDTH: u32 = 960;
/// Shown in the chat when the GIF is at most this big; larger ones are only
/// saved to disk.
const INLINE_MAX: usize = 6 * 1024 * 1024;

struct Recording {
    session: String,
    started: Instant,
    frames: Arc<Mutex<Vec<(Duration, String)>>>,
    task: tokio::task::JoinHandle<()>,
}

fn current() -> &'static Mutex<Option<Recording>> {
    static R: std::sync::OnceLock<Mutex<Option<Recording>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

pub async fn start(state: &AppState, slot: &SessionSlot) -> Result<String, String> {
    if current().lock().map(|r| r.is_some()).unwrap_or(false) {
        return Err("a recording is already running; call browser_record_stop first".into());
    }
    // The browser must be up for there to be anything to record.
    state
        .browser
        .ensure_started()
        .await
        .map_err(|e| e.to_string())?;
    let (mut rx, first) = state.browser.subscribe();
    let started = Instant::now();
    let frames: Arc<Mutex<Vec<(Duration, String)>>> = Arc::new(Mutex::new(Vec::new()));
    // The current frame is the opening shot.
    for ev in first {
        if let mira_browser::LiveEvent::Frame { jpeg_base64, .. } = ev {
            if let Ok(mut f) = frames.lock() {
                f.push((Duration::ZERO, jpeg_base64));
            }
        }
    }
    let sink = frames.clone();
    let task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(mira_browser::LiveEvent::Frame { jpeg_base64, .. }) => {
                    let at = started.elapsed();
                    if at > MAX_LEN {
                        break;
                    }
                    let Ok(mut f) = sink.lock() else { break };
                    // Too soon after the last kept frame: replace it, so the
                    // newest picture wins without raising the rate. (The
                    // opening shot is always kept.)
                    let replace = f.len() > 1
                        && f.last()
                            .is_some_and(|(t, _)| at.saturating_sub(*t) < MIN_GAP);
                    match f.last_mut() {
                        Some((_, img)) if replace => *img = jpeg_base64,
                        _ => f.push((at, jpeg_base64)),
                    }
                }
                Ok(mira_browser::LiveEvent::Closed) => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    if let Ok(mut r) = current().lock() {
        *r = Some(Recording {
            session: slot.id.to_string(),
            started,
            frames,
            task,
        });
    }
    Ok("Recording the browser. Do what you want captured, then call browser_record_stop.".into())
}

pub async fn stop(state: &AppState, slot: &SessionSlot) -> Result<String, String> {
    let rec = current()
        .lock()
        .ok()
        .and_then(|mut r| r.take())
        .ok_or("no recording is running")?;
    rec.task.abort();
    let total = rec.started.elapsed().min(MAX_LEN);
    let frames = std::mem::take(&mut *rec.frames.lock().map_err(|_| "recording lost")?);
    if frames.is_empty() {
        return Err("nothing was recorded: the page never painted while recording".into());
    }
    let gif = tokio::task::spawn_blocking(move || encode(&frames, total))
        .await
        .map_err(|e| e.to_string())??;
    let dir = state
        .store
        .as_ref()
        .and_then(|s| s.agent_log_path(&slot.id))
        .and_then(|p| p.parent().map(|d| d.join("recordings").join(&rec.session)))
        .unwrap_or_else(|| std::env::temp_dir().join("mira-recordings"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let path = dir.join(format!("browser-{stamp}.gif"));
    std::fs::write(&path, &gif).map_err(|e| e.to_string())?;
    let secs = total.as_secs_f64();
    let shown = gif.len() <= INLINE_MAX;
    if shown {
        let data = base64::engine::general_purpose::STANDARD.encode(&gif);
        let _ = slot.events_tx.send(ServerMsg::HtmlRender {
            id: uuid::Uuid::new_v4().simple().to_string(),
            title: format!("Browser recording · {secs:.0}s"),
            html: format!(
                "<!doctype html><meta charset=utf-8><style>html,body{{margin:0;background:#111}}img{{display:block;max-width:100%;margin:auto}}</style><img alt=\"Browser recording\" src=\"data:image/gif;base64,{data}\">"
            ),
        });
    }
    Ok(format!(
        "Saved a {secs:.0}s recording ({} KB) to {}.{}",
        gif.len() / 1024,
        path.display(),
        if shown {
            " It's shown in the chat."
        } else {
            " Too large to show in the chat."
        }
    ))
}

/// Frames → GIF, each held until the next frame (the last until `total`).
fn encode(frames: &[(Duration, String)], total: Duration) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 20);
        enc.set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        for (i, (at, b64)) in frames.iter().enumerate() {
            let next = frames.get(i + 1).map(|(t, _)| *t).unwrap_or(total);
            let hold = next.saturating_sub(*at).max(Duration::from_millis(60));
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| e.to_string())?;
            let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?;
            let img = if img.width() > MAX_WIDTH {
                img.resize(MAX_WIDTH, u32::MAX, image::imageops::FilterType::Triangle)
            } else {
                img
            };
            let delay = Delay::from_saturating_duration(hold);
            enc.encode_frame(Frame::from_parts(img.to_rgba8(), 0, 0, delay))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32, shade: u8) -> String {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([shade, 0, 255 - shade]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
    }

    #[test]
    fn frames_become_an_animated_gif_scaled_to_fit() {
        let frames = vec![
            (Duration::ZERO, jpeg(1600, 900, 10)),
            (Duration::from_millis(400), jpeg(1600, 900, 200)),
        ];
        let gif = encode(&frames, Duration::from_secs(2)).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        let decoded = image::load_from_memory_with_format(&gif, image::ImageFormat::Gif).unwrap();
        assert_eq!(decoded.width(), MAX_WIDTH);
    }
}
