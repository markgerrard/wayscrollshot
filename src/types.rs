use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use image::RgbaImage;

/// User-selected capture region in global compositor coordinates.
#[derive(Clone, Debug)]
pub struct Region {
    pub raw: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Shared run-state flags for capture worker and command loop.
#[derive(Default)]
pub struct Control {
    running: AtomicBool,
    paused: AtomicBool,
    auto_scroll: AtomicBool,
}

impl Control {
    /// Creates a running, unpaused control state.
    pub fn new(auto_scroll: bool) -> Self {
        Self {
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            auto_scroll: AtomicBool::new(auto_scroll),
        }
    }

    pub fn toggle_pause(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
    pub fn enable_auto_scroll(&self) {
        self.auto_scroll.store(true, Ordering::Relaxed);
    }
    pub fn is_auto_scroll(&self) -> bool {
        self.auto_scroll.load(Ordering::Relaxed)
    }

    /// Requests worker shutdown.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Returns whether the worker should continue running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct StitchStats {
    pub frame_count: u32,
    pub total_height: u32,
    pub last_append: u32,
}

#[derive(Default)]
pub struct StitchState {
    pub full_image: Option<Arc<RgbaImage>>,
    pub preview: Option<PreviewImage>,
    pub stats: StitchStats,
    pub revision: u64,
    pub last_message: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum UserCommand {
    Save,
    Copy,
    Cloud,
    EnableAuto,
    Cancel,
    TogglePause,
}

#[derive(Clone, Debug)]
pub enum LayerMessage {
    Preview(PreviewImage),
    Paused(bool),
    Auto(bool),
    Dimensions(u32, u32),
}
