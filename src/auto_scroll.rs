use crate::types::Region;
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
    center: (i64, i64),
    pixels_per_tick: f64,
    last_ticks: i32,
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
            center: (
                i64::from(region.x) + i64::from(region.w) / 2,
                i64::from(region.y) + i64::from(region.h) / 2,
            ),
            conn,
            queue,
            pointer,
        })
    }
    pub fn observe(&mut self, pixels: u32) {
        self.pixels_per_tick = (pixels as f64 / self.last_ticks.max(1) as f64).clamp(20.0, 500.0);
    }
    pub fn step(&mut self, target_pixels: u32) -> Result<()> {
        self.last_ticks =
            ((target_pixels as f64 / self.pixels_per_tick).floor() as i32).clamp(1, 8);
        let coords = cursor_position()?;
        anyhow::ensure!(
            (coords.0 - self.center.0).abs() <= 24 && (coords.1 - self.center.1).abs() <= 24,
            "Pointer moved; automatic scrolling stopped"
        );
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        self.pointer.axis_discrete(
            0,
            wl_pointer::Axis::VerticalScroll,
            15.0 * f64::from(self.last_ticks),
            self.last_ticks,
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
