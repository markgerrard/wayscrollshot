use crate::types::{Control, Region};
use anyhow::{Context, Result};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_pointer, wl_registry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct State;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);

pub struct AutoScroller {
    region: Region,
    pixels_per_tick: f64,
    last_ticks: i32,
    max_ticks: i32,
    conn: Connection,
    queue: wayland_client::EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
}
impl AutoScroller {
    pub fn new(region: &Region) -> Result<Self> {
        let conn = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .context("Compositor does not support automatic scrolling")?;
        let pointer = manager.create_virtual_pointer(None, &qh, ());
        // Hyprland uses global logical coordinates, including negative monitor origins.
        let expr = format!(
            "hl.dispatch(hl.dsp.cursor.move({{x={},y={}}}))",
            i64::from(region.x) + i64::from(region.w) / 2,
            i64::from(region.y) + i64::from(region.h) / 2
        );
        let result = std::process::Command::new("hyprctl")
            .args(["eval", &expr])
            .output()?;
        anyhow::ensure!(
            result.status.success(),
            "Could not position pointer: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        queue.roundtrip(&mut State)?;
        Ok(Self {
            pixels_per_tick: 120.0,
            last_ticks: 1,
            max_ticks: 8,
            region: region.clone(),
            conn,
            queue,
            pointer,
        })
    }
    pub fn observe(&mut self, pixels: u32) {
        self.pixels_per_tick = (pixels as f64 / self.last_ticks.max(1) as f64).clamp(20.0, 500.0);
    }
    pub fn step(&mut self, target_pixels: u32, control: &Control) -> Result<()> {
        self.last_ticks =
            ((target_pixels as f64 / self.pixels_per_tick).floor() as i32).clamp(1, self.max_ticks);
        self.send_ticks(self.last_ticks, control)
    }
    /// Backtrack part of a rejected jump while preserving the stitcher's anchor.
    pub fn retry_smaller(&mut self, control: &Control) -> Result<bool> {
        let Some((back, remaining)) = reduced_jump(self.last_ticks) else {
            return Ok(false);
        };
        self.send_ticks(-back, control)?;
        self.last_ticks = remaining;
        self.max_ticks = self.max_ticks.min(remaining);
        Ok(true)
    }
    fn send_ticks(&mut self, ticks: i32, control: &Control) -> Result<()> {
        if !wait_until_scrollable(&self.region, control, cursor_position)? {
            return Ok(());
        }
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        self.pointer.axis_discrete(
            0,
            wl_pointer::Axis::VerticalScroll,
            15.0 * f64::from(ticks),
            ticks,
        );
        self.pointer.frame();
        self.conn.flush()?;
        self.queue.roundtrip(&mut State)?;
        Ok(())
    }
}
impl Drop for AutoScroller {
    fn drop(&mut self) {
        self.pointer.destroy();
        let _ = self.conn.flush();
    }
}

fn wait_until_scrollable(
    region: &Region,
    control: &Control,
    mut query: impl FnMut() -> Result<(i64, i64)>,
) -> Result<bool> {
    while control.is_running() {
        if !control.is_paused() && pointer_inside(region, query()?) {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(false)
}

fn pointer_inside(region: &Region, (x, y): (i64, i64)) -> bool {
    let left = i64::from(region.x);
    let top = i64::from(region.y);
    x >= left && x < left + i64::from(region.w) && y >= top && y < top + i64::from(region.h)
}

fn reduced_jump(ticks: i32) -> Option<(i32, i32)> {
    if ticks <= 1 {
        return None;
    }
    let remaining = ticks / 2;
    Some((ticks - remaining, remaining))
}

fn cursor_position() -> Result<(i64, i64)> {
    let result = std::process::Command::new("hyprctl")
        .arg("cursorpos")
        .output()?;
    anyhow::ensure!(result.status.success(), "Hyprland cursor query failed");
    let text = String::from_utf8(result.stdout)?;
    parse_cursor_position(&text).context("Could not read Hyprland cursor position")
}

fn parse_cursor_position(text: &str) -> Option<(i64, i64)> {
    let (x, y) = text.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leaving_then_returning_to_crop_resumes_scrolling() {
        let r = Region {
            raw: String::new(),
            x: 100,
            y: 100,
            w: 900,
            h: 700,
        };
        let control = Control::new(false);
        let mut calls = 0;
        assert!(wait_until_scrollable(&r, &control, || {
            calls += 1;
            Ok(if calls == 1 { (0, 0) } else { (500, 400) })
        })
        .unwrap());
        assert_eq!(calls, 2);
    }
    #[test]
    fn finishing_capture_interrupts_pointer_wait() {
        let r = Region {
            raw: String::new(),
            x: 100,
            y: 100,
            w: 900,
            h: 700,
        };
        let control = std::sync::Arc::new(Control::new(false));
        let stop = control.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            stop.stop();
        });
        assert!(!wait_until_scrollable(&r, &control, || Ok((0, 0))).unwrap());
        worker.join().unwrap();
    }
    #[test]
    fn pointer_can_move_across_the_capture_without_stopping() {
        let r = Region {
            raw: String::new(),
            x: 100,
            y: 100,
            w: 900,
            h: 700,
        };
        assert!(pointer_inside(&r, (110, 110)));
        assert!(pointer_inside(&r, (990, 790)));
        assert!(!pointer_inside(&r, (1000, 400)));
    }
    #[test]
    fn pointer_bounds_support_negative_monitor_coordinates() {
        let r = Region {
            raw: String::new(),
            x: -1920,
            y: -100,
            w: 1000,
            h: 700,
        };
        assert!(pointer_inside(&r, (-1900, 0)));
        assert!(!pointer_inside(&r, (-900, 0)));
    }
    #[test]
    fn recovery_keeps_a_positive_smaller_offset_from_original_anchor() {
        assert_eq!(reduced_jump(4), Some((2, 2)));
        assert_eq!(reduced_jump(3), Some((2, 1)));
        assert_eq!(reduced_jump(2), Some((1, 1)));
        assert_eq!(reduced_jump(1), None);
    }
    #[test]
    fn parses_cursor_coordinates_and_rejects_command_errors() {
        assert_eq!(parse_cursor_position("123, 456\n"), Some((123, 456)));
        assert_eq!(parse_cursor_position("-1200, -25\n"), Some((-1200, -25)));
        assert_eq!(parse_cursor_position("unknown request\n"), None);
    }
    #[test]
    #[ignore = "requires a running Hyprland desktop"]
    fn live_cursor_query() {
        cursor_position().expect("real desktop cursor query must succeed");
    }
}
