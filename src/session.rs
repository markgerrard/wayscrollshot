use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;

use crate::capture::{
    capture_frame, read_region_from_stdin, region_from_slurp_output, select_region,
};
use crate::cli::{Algorithm, Args};
use crate::output::{copy_to_clipboard, save_image};
use crate::stitch::{build_preview, init_opencv_runtime, MatchConfig, StitchOutcome, Stitcher};
use crate::types::{Control, LayerMessage, Region, StitchState, UserCommand};

/// Runs one interactive capture session from region selection to final action.
pub fn run(args: Args) -> Result<()> {
    let Some(socket) = crate::control_socket::CaptureSocket::open(args.toggle)? else {
        return Ok(());
    };
    if !is_wayland_session() {
        bail!("Wayland session required");
    }
    let region = resolve_region(&args)?;
    anyhow::ensure!(
        region.w >= 32 && region.h >= 160,
        "Select an area at least 32 pixels wide and 160 pixels high"
    );
    let control = Arc::new(Control::new());
    let state = Arc::new(Mutex::new(StitchState::default()));
    let (tx, rx) = mpsc::channel();
    let mut mask = if args.no_border {
        None
    } else {
        Some(crate::region_overlay::RegionOverlay::new(region.clone())?)
    };
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut live = if args.no_preview {
        None
    } else {
        crate::overlay::LayerShellOverlay::new_live(ui_tx, region.clone(), args.preview_width)?
    };
    let mut revision = 0;
    let mut action = None;
    let worker = spawn_capture_worker(
        region.clone(),
        control.clone(),
        state.clone(),
        args.clone(),
        tx,
    );
    let reason = loop {
        if let Some(overlay) = live.as_ref() {
            let st = state.lock().expect("state lock");
            if st.revision != revision {
                revision = st.revision;
                if let Some(img) = st.full_image.as_ref() {
                    overlay.send(LayerMessage::Preview(build_preview(img, overlay.width)));
                }
            }
        }
        match ui_rx.try_recv() {
            Ok(UserCommand::TogglePause) => {
                control.toggle_pause();
                if let Some(o) = live.as_ref() {
                    o.send(LayerMessage::Paused(control.is_paused()));
                }
            }
            Ok(command) => {
                action = Some(command);
                break "Capture finished".to_string();
            }
            Err(_) => {}
        }
        if socket.finish_requested() {
            break "Capture finished".to_string();
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(reason) => break reason,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break "Capture worker stopped unexpectedly".to_string(),
        }
    };
    control.stop();
    worker
        .join()
        .map_err(|_| anyhow!("capture worker panicked"))?;
    if let Some(overlay) = live.as_mut() {
        overlay.stop();
    }
    if let Some(overlay) = mask.as_mut() {
        overlay.stop();
    }
    if matches!(action, Some(UserCommand::Cancel)) {
        return Ok(());
    }
    let img = take_snapshot(&state).context(reason.clone())?;
    let mut clipboard =
        matches!(action, Some(UserCommand::Copy)) || (action.is_none() && args.clipboard);
    if !args.no_preview && action.is_none() {
        let _ = std::process::Command::new("notify-send")
            .args(["Capture ready to review", &reason])
            .status();
        let (tx, rx) = mpsc::channel();
        let mut review = crate::overlay::LayerShellOverlay::new(tx, region, args.preview_width)?;
        review.send(LayerMessage::Preview(build_preview(
            &img,
            args.preview_width,
        )));
        review.send(LayerMessage::Paused(true));
        let cancelled = loop {
            if socket.finish_requested() {
                break false;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(UserCommand::Save) => {
                    clipboard = false;
                    break false;
                }
                Ok(UserCommand::Copy) => {
                    clipboard = true;
                    break false;
                }
                Ok(UserCommand::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => break true,
                _ => {}
            }
        };
        review.stop();
        if cancelled {
            return Ok(());
        }
    }
    let message = if clipboard {
        copy_to_clipboard(img)?;
        format!("{reason}. Copied to clipboard")
    } else {
        let path = save_image(img, args.output)?;
        println!("{}", path.display());
        format!("{reason}. Saved to {}", path.display())
    };
    let _ = std::process::Command::new("notify-send")
        .args(["Scrolling capture", &message])
        .status();
    Ok(())
}

fn resolve_region(args: &Args) -> Result<Region> {
    match args.slurp_output() {
        Some(raw) if raw.trim() == "-" => read_region_from_stdin().context("failed to read region"),
        Some(raw) => region_from_slurp_output(&raw),
        None => select_region(),
    }
}

/// Returns the latest stitched image snapshot.
fn take_snapshot(state: &Arc<Mutex<StitchState>>) -> Result<Arc<RgbaImage>> {
    let state = state.lock().expect("state lock");
    state
        .full_image
        .clone()
        .ok_or_else(|| anyhow!("no frames captured yet"))
}

/// Detects whether current desktop session is Wayland.
fn is_wayland_session() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return true;
    }
    matches!(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        Some("wayland")
    )
}

/// Spawns background capture + stitching worker.
fn spawn_capture_worker(
    region: Region,
    control: Arc<Control>,
    state: Arc<Mutex<StitchState>>,
    args: Args,
    done: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let result = capture_loop(&region, &control, &state, &args);
        let message = match result {
            Ok(msg) => msg,
            Err(err) => format!("Stopped: {err}"),
        };
        log::info!("{message}");
        let _ = done.send(message);
    })
}

fn capture_loop(
    region: &Region,
    control: &Control,
    state: &Arc<Mutex<StitchState>>,
    args: &Args,
) -> Result<String> {
    let mut auto = if args.auto_scroll {
        Some(crate::auto_scroll::AutoScroller::new(region)?)
    } else {
        None
    };
    let config = MatchConfig {
        min_overlap: (region.h / 12).max(48),
        accept_diff: 3.5,
        min_append: 2,
        approx_diff: 0.5,
        algorithm: args.algorithm,
        match_width: args.preview_width.max(200),
    };
    if matches!(args.algorithm, Algorithm::OpenCvOrb) {
        init_opencv_runtime();
    }
    let mut stitcher = Stitcher::new(config);
    let mut candidate: Option<RgbaImage> = None;
    let mut stable_since = Instant::now();
    let mut submitted = false;
    let mut unchanged_steps = 0;
    let mut overlap_retries = 0;
    let started = Instant::now();
    let mut last_accepted = Instant::now();
    while control.is_running() {
        thread::sleep(Duration::from_millis(50));
        if control.is_paused() {
            candidate = None;
            submitted = false;
            stable_since = Instant::now();
            last_accepted = Instant::now();
            continue;
        }
        if auto.is_some() && started.elapsed() > Duration::from_secs(180) {
            return Ok("Time limit reached; partial capture".into());
        }
        let frame = capture_frame(region)?;
        let stable = candidate
            .as_ref()
            .is_some_and(|prev| frames_settled(prev, &frame));
        if !stable {
            stable_since = Instant::now();
            submitted = false;
        }
        candidate = Some(frame.clone());
        if !stable || stable_since.elapsed() < Duration::from_millis(args.settle_ms) || submitted {
            if auto.is_some() && last_accepted.elapsed() > Duration::from_secs(10) {
                return Ok("Page did not settle; partial capture".into());
            }
            continue;
        }
        submitted = true;
        let outcome = stitcher.push_frame(frame);
        if let StitchOutcome::Appended { added } = &outcome {
            if let Some(scroller) = auto.as_mut() {
                scroller.observe(*added);
            }
            log::info!("Appended {added} pixels");
        }
        match outcome {
            StitchOutcome::FirstFrame | StitchOutcome::Appended { .. } => {
                unchanged_steps = 0;
                overlap_retries = 0;
                last_accepted = Instant::now();
                apply_state_update(
                    state,
                    &stitcher,
                    "Captured settled content".into(),
                    None,
                    args.preview_width,
                );
            }
            StitchOutcome::NoProgress => {
                unchanged_steps += 1;
                last_accepted = Instant::now();
            }
            StitchOutcome::NoMatch => {
                if let Some(scroller) = auto.as_mut() {
                    overlap_retries += 1;
                    if overlap_retries == 1 {
                        log::warn!(
                            "Overlap rejected; waiting for delayed rendering before retrying"
                        );
                    } else if overlap_retries <= 4 && scroller.retry_smaller(control)? {
                        log::warn!("Overlap rejected; backed up and reduced future scroll steps");
                    } else {
                        return Ok("Could not align this section after retrying with smaller steps; partial capture".into());
                    }
                    // Retry against the last accepted frame, never an unverified one.
                    thread::sleep(Duration::from_millis(700));
                    candidate = None;
                    submitted = false;
                    stable_since = Instant::now();
                    last_accepted = Instant::now();
                    continue;
                }
                log::warn!("Overlap rejected; scroll back toward the last accepted content");
            }
        }
        if let Some(scroller) = auto.as_mut() {
            if unchanged_steps >= 4 {
                return Ok("Reached end of scrolling content".into());
            }
            if stitcher.stats().total_height as u64 * region.w as u64 * 4 > 192 * 1024 * 1024 {
                return Ok("Image size limit reached; partial capture".into());
            }
            scroller.step(region.h * args.scroll_percent / 100, control)?;
            // Allow scroll animations to start before looking for a settled frame.
            thread::sleep(Duration::from_millis(60));
            candidate = None;
            submitted = false;
            stable_since = Instant::now();
        }
    }
    Ok("Capture finished".into())
}

fn frames_settled(a: &RgbaImage, b: &RgbaImage) -> bool {
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let mut changed = 0usize;
    let mut total = 0usize;
    for y in (0..a.height()).step_by(3) {
        for x in (0..a.width()).step_by(3) {
            let p = a.get_pixel(x, y);
            let q = b.get_pixel(x, y);
            if (0..3).any(|c| p[c].abs_diff(q[c]) > 3) {
                changed += 1;
            }
            total += 1;
        }
    }
    total > 0 && changed as f64 / (total as f64) < 0.002
}

/// Applies a successful stitch update and publishes preview frame.
fn apply_state_update(
    state: &Arc<Mutex<StitchState>>,
    stitcher: &Stitcher,
    message: String,
    preview_tx: Option<&mpsc::Sender<LayerMessage>>,
    preview_width: u32,
) {
    let preview = preview_tx.and_then(|_| {
        stitcher
            .full_image()
            .map(|img| build_preview(img.as_ref(), preview_width))
    });
    if let (Some(tx), Some(preview)) = (preview_tx, preview.as_ref()) {
        let _ = tx.send(LayerMessage::Preview(preview.clone()));
    }
    let mut st = state.lock().expect("state lock");
    st.full_image = stitcher.full_image();
    st.preview = preview;
    st.stats = stitcher.stats();
    st.last_message = message;
    st.last_error = None;
    st.revision = st.revision.wrapping_add(1);
}

#[cfg(test)]
mod settling_tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn waits_for_local_animation_to_settle() {
        let a = RgbaImage::from_pixel(300, 300, Rgba([255, 255, 255, 255]));
        let mut b = a.clone();
        for y in 100..150 {
            for x in 100..200 {
                b.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!frames_settled(&a, &b));
        assert!(frames_settled(&b, &b));
    }
}
