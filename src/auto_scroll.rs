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
            center: (
                i64::from(region.x) + i64::from(region.w) / 2,
                i64::from(region.y) + i64::from(region.h) / 2,
            ),
            conn,
            queue,
            pointer,
        })
    }
    pub fn step(&mut self) -> Result<()> {
        let result = std::process::Command::new("hyprctl")
            .arg("getcursorpos")
            .output()?;
        let text = String::from_utf8(result.stdout)?;
        let coords: Vec<i64> = text
            .trim()
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        anyhow::ensure!(
            coords.len() == 2
                && (coords[0] - self.center.0).abs() <= 24
                && (coords[1] - self.center.1).abs() <= 24,
            "Pointer moved; automatic scrolling stopped"
        );
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        self.pointer
            .axis_discrete(0, wl_pointer::Axis::VerticalScroll, 15.0, 1);
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
