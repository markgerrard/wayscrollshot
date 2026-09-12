//! Native selection surface; desktop geometry is snapshotted before taking focus.
use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_region, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use crate::overlay::placement::{find_output_by_id, probe_output_rects, OutputRect};
use crate::{types::Region, ui};
use serde_json::Value;

struct Selector {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    configured: bool,
    dirty: bool,
    exit: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: bool,
    pointer: Option<wl_pointer::WlPointer>,
    output: OutputRect,
    windows: Vec<Region>,
    position: (f64, f64),
    selection: Option<Region>,
    drag: Option<((f64, f64), Option<Region>, u8)>,
    result: Option<Region>,
}
fn rectangle(x: i32, y: i32, w: u32, h: u32) -> Region {
    Region {
        x,
        y,
        w,
        h,
        raw: format!("{x},{y} {w}x{h}"),
    }
}
fn drag_rect(a: (f64, f64), b: (f64, f64)) -> Region {
    rectangle(
        a.0.min(b.0).round() as i32,
        a.1.min(b.1).round() as i32,
        (a.0 - b.0).abs().round() as u32,
        (a.1 - b.1).abs().round() as u32,
    )
}
impl Selector {
    fn request_redraw(&mut self, qh: &QueueHandle<Self>) {
        if self.configured {
            self.draw(qh);
        }
    }
    fn choose_screen(&mut self) {
        self.selection = Some(rectangle(0, 0, self.width, self.height));
    }
    fn choose_window(&mut self) {
        // A freshly created virtual keyboard can deliver a pointer enter at (0,0).
        // Query the compositor at selection time instead of trusting that synthetic event.
        if let Ok(cursor) = query("cursorpos") {
            if let (Some(x), Some(y)) = (cursor["x"].as_i64(), cursor["y"].as_i64()) {
                self.position = (
                    (x - self.output.x as i64) as f64,
                    (y - self.output.y as i64) as f64,
                );
            }
        }
        let x = self.position.0 as i32 + self.output.x;
        let y = self.position.1 as i32 + self.output.y;
        self.selection = self
            .windows
            .iter()
            .find(|r| x >= r.x && y >= r.y && x < r.x + r.w as i32 && y < r.y + r.h as i32)
            .map(|r| {
                let left = (r.x - self.output.x).max(0);
                let top = (r.y - self.output.y).max(0);
                let right = (r.x - self.output.x + r.w as i32).min(self.width as i32);
                let bottom = (r.y - self.output.y + r.h as i32).min(self.height as i32);
                rectangle(
                    left,
                    top,
                    (right - left).max(0) as u32,
                    (bottom - top).max(0) as u32,
                )
            });
    }
    fn finish(&mut self) {
        if let Some(r) = &self.selection {
            if r.w >= 32 && r.h >= 160 {
                self.result = Some(rectangle(
                    r.x + self.output.x,
                    r.y + self.output.y,
                    r.w,
                    r.h,
                ));
                self.exit = true;
            }
        }
    }
    fn press(&mut self) {
        let (x, y) = self.position;
        let bx = (self.width as f64 - 440.) / 2.;
        if y >= self.height as f64 - 84.
            && y <= self.height as f64 - 36.
            && x >= bx
            && x < bx + 440.
        {
            if x < bx + 110. {
                self.exit = true;
            } else if x < bx + 280. {
                self.choose_screen();
            } else {
                self.finish();
            }
            return;
        }
        let mut edges = 0;
        if let Some(r) = &self.selection {
            let right = (r.x + r.w as i32) as f64;
            let bottom = (r.y + r.h as i32) as f64;
            if x >= r.x as f64 - 12.
                && x <= right + 12.
                && y >= r.y as f64 - 12.
                && y <= bottom + 12.
            {
                if (x - r.x as f64).abs() < 12. {
                    edges |= 1;
                }
                if (x - right).abs() < 12. {
                    edges |= 2;
                }
                if (y - r.y as f64).abs() < 12. {
                    edges |= 4;
                }
                if (y - bottom).abs() < 12. {
                    edges |= 8;
                }
            }
        }
        self.drag = Some((self.position, self.selection.clone(), edges));
        if edges == 0 {
            self.selection = Some(drag_rect(self.position, self.position));
        }
    }
    fn motion(&mut self) {
        let Some((start, old, edges)) = &self.drag else {
            return;
        };
        let p = (
            self.position.0.clamp(0., self.width as f64),
            self.position.1.clamp(0., self.height as f64),
        );
        if *edges == 0 {
            self.selection = Some(drag_rect(*start, p));
        } else if let Some(r) = old {
            let mut a = (r.x as f64, r.y as f64);
            let mut b = ((r.x + r.w as i32) as f64, (r.y + r.h as i32) as f64);
            if edges & 1 != 0 {
                a.0 = p.0;
            }
            if edges & 2 != 0 {
                b.0 = p.0;
            }
            if edges & 4 != 0 {
                a.1 = p.1;
            }
            if edges & 8 != 0 {
                b.1 = p.1;
            }
            self.selection = Some(drag_rect(a, b));
        }
    }
    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }
        let mut p = tiny_skia::Pixmap::new(self.width, self.height).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 105));
        if let Some(r) = &self.selection {
            for y in r.y.max(0) as u32..(r.y + r.h as i32).max(0).min(self.height as i32) as u32 {
                let left = r.x.max(0).min(self.width as i32) as u32;
                let right = (r.x + r.w as i32).max(0).min(self.width as i32) as u32;
                p.data_mut()[((y * self.width + left) * 4) as usize
                    ..((y * self.width + right) * 4) as usize]
                    .fill(0);
            }
            let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            for (a, b, c, d) in [
                (x, y, w, 1.),
                (x, y, 1., h),
                (x, y + h - 1., w, 1.),
                (x + w - 1., y, 1., h),
            ] {
                ui::rounded(&mut p, a, b, c, d, 0., [255, 255, 255, 255]);
            }
            for (a, b) in [
                (x, y),
                (x + w / 2., y),
                (x + w, y),
                (x, y + h / 2.),
                (x + w, y + h / 2.),
                (x, y + h),
                (x + w / 2., y + h),
                (x + w, y + h),
            ] {
                ui::rounded(&mut p, a - 4., b - 4., 8., 8., 2., [255, 255, 255, 255]);
            }
            let label = format!("{} × {}", r.w, r.h);
            let tw = ui::text_width(&label, 15.);
            let lx = (x + w - tw - 36.).clamp(4., (self.width as f32 - tw - 28.).max(4.));
            let ly = if y > 48. {
                y - 40.
            } else {
                (y + 12.).min(self.height as f32 - 40.)
            };
            ui::rounded(&mut p, lx, ly, tw + 24., 28., 8., [28, 29, 34, 245]);
            ui::text(&mut p, &label, lx + 12., ly + 5., 15., [255, 255, 255, 255]);
        }
        let title = if self.selection.is_some() {
            "Adjust the edges, then capture"
        } else {
            "Drag to select an area"
        };
        let hint = "Space  Window   F  Screen   Enter  Capture   Esc  Cancel";
        let pw = 520.;
        let px = (self.width as f32 - pw) / 2.;
        ui::rounded(&mut p, px, 40., pw, 82., 16., [28, 29, 34, 245]);
        ui::text(
            &mut p,
            title,
            (self.width as f32 - ui::text_width(title, 23.)) / 2.,
            54.,
            23.,
            [255, 255, 255, 255],
        );
        ui::text(
            &mut p,
            hint,
            (self.width as f32 - ui::text_width(hint, 15.)) / 2.,
            88.,
            15.,
            [188, 190, 199, 255],
        );
        let bx = (self.width as f32 - 440.) / 2.;
        let by = self.height as f32 - 84.;
        ui::rounded(&mut p, bx - 8., by - 8., 456., 64., 18., [28, 29, 34, 245]);
        for (offset, w, label, primary) in [
            (0., 106., "Cancel", false),
            (110., 166., "Full screen", false),
            (280., 160., "Capture", true),
        ] {
            ui::rounded(
                &mut p,
                bx + offset,
                by,
                w,
                48.,
                12.,
                if primary {
                    [67, 99, 154, 255]
                } else {
                    [54, 55, 62, 255]
                },
            );
            ui::icon(
                &mut p,
                if primary {
                    0
                } else if offset == 0. {
                    4
                } else {
                    5
                },
                bx + offset + 8.,
                by + 14.,
                20,
            );
            ui::text(
                &mut p,
                label,
                bx + offset + 8. + (w - ui::text_width(label, 17.)) / 2.,
                by + 14.,
                17.,
                if primary {
                    [255, 255, 255, 255]
                } else {
                    [245, 245, 247, 255]
                },
            );
        }
        let needed = (self.width * self.height * 4) as usize;
        if self.pool.len() < needed {
            if self.pool.resize(needed).is_err() {
                return;
            }
        }
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            self.width as i32,
            self.height as i32,
            self.width as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        ui::to_canvas(&p, canvas, self.width, 0);
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.width as i32, self.height as i32);
        if buffer.attach_to(self.layer.wl_surface()).is_ok() {
            self.layer.commit();
        }
    }
}
impl CompositorHandler for Selector {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Selector {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.request_redraw(qh);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.request_redraw(qh);
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.request_redraw(qh);
    }
}

impl LayerShellHandler for Selector {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 > 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 > 0 {
            self.height = configure.new_size.1;
        }
        self.configured = true;
        self.draw(qh);
    }
}

impl SeatHandler for Selector {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let keyboard = self
                .seat_state
                .get_keyboard(qh, &seat, None)
                .expect("failed to create keyboard");
            self.keyboard = Some(keyboard);
        }

        if capability == Capability::Pointer && self.pointer.is_none() {
            let pointer = self
                .seat_state
                .get_pointer(qh, &seat)
                .expect("failed to create pointer");
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }

        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Selector {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = true;
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = false;
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        match event.keysym {
            Keysym::Escape => self.exit = true,
            Keysym::Return => self.finish(),
            Keysym::space => self.choose_window(),
            Keysym::f | Keysym::F => self.choose_screen(),
            _ => {}
        }
        self.draw(qh);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for Selector {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }

            self.position = event.position;
            match event.kind {
                PointerEventKind::Press { button: 272, .. } => {
                    self.press();
                    self.dirty = true;
                }
                PointerEventKind::Release { button: 272, .. } => {
                    self.drag = None;
                }
                PointerEventKind::Motion { .. } if self.drag.is_some() => {
                    self.motion();
                    self.dirty = true;
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Selector {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Selector);
delegate_output!(Selector);
delegate_shm!(Selector);
delegate_seat!(Selector);
delegate_keyboard!(Selector);
delegate_pointer!(Selector);
delegate_layer!(Selector);
delegate_registry!(Selector);

impl ProvidesRegistryState for Selector {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState, SeatState);
}

impl wayland_client::Dispatch<wl_region::WlRegion, ()> for Selector {
    fn event(
        _state: &mut Self,
        _proxy: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

fn query(command: &str) -> Result<Value> {
    let out = std::process::Command::new("hyprctl")
        .args([command, "-j"])
        .output()?;
    anyhow::ensure!(out.status.success(), "Cannot read desktop geometry");
    Ok(serde_json::from_slice(&out.stdout)?)
}
pub fn select() -> Result<Option<Region>> {
    let cursor = query("cursorpos")?;
    let cx = cursor["x"].as_i64().unwrap_or(0);
    let cy = cursor["y"].as_i64().unwrap_or(0);
    let rects = probe_output_rects()?;
    let output = *rects
        .iter()
        .find(|r| {
            cx >= r.x as i64
                && cy >= r.y as i64
                && cx < (r.x + r.width) as i64
                && cy < (r.y + r.height) as i64
        })
        .or(rects.first())
        .context("No screen available")?;
    let monitors = query("monitors")?;
    let workspace = monitors
        .as_array()
        .context("No monitors")?
        .iter()
        .find(|m| {
            m["x"].as_i64() == Some(output.x as i64) && m["y"].as_i64() == Some(output.y as i64)
        })
        .map(|m| m["activeWorkspace"]["id"].clone());
    let clients = query("clients")?;
    let mut clients: Vec<_> = clients
        .as_array()
        .context("No windows")?
        .iter()
        .filter(|c| {
            c["mapped"] == true
                && c["hidden"] != true
                && (Some(c["workspace"]["id"].clone()) == workspace || c["pinned"] == true)
        })
        .collect();
    clients.sort_by_key(|c| c["focusHistoryID"].as_i64().unwrap_or(999));
    let windows = clients
        .iter()
        .filter_map(|c| {
            Some(rectangle(
                c["at"][0].as_i64()? as i32,
                c["at"][1].as_i64()? as i32,
                c["size"][0].as_u64()? as u32,
                c["size"][1].as_u64()? as u32,
            ))
        })
        .collect();
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let output_state = OutputState::new(&globals, &qh);
    let selected = find_output_by_id(&output_state, output.id);
    let layer = shell.create_layer_surface(
        &qh,
        compositor.create_surface(&qh),
        Layer::Overlay,
        Some("wayscrollshot-selection"),
        selected.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::BOTTOM | Anchor::RIGHT);
    let input = compositor.wl_compositor().create_region(&qh, ());
    input.add(0, 0, output.width, output.height);
    layer.wl_surface().set_input_region(Some(&input));
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_size(0, 0);
    layer.commit();
    let pool = SlotPool::new((output.width * output.height * 4) as usize, &shm)?;
    let mut state = Selector {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state,
        shm,
        pool,
        layer,
        width: output.width as u32,
        height: output.height as u32,
        configured: false,
        dirty: false,
        exit: false,
        keyboard: None,
        keyboard_focus: false,
        pointer: None,
        output,
        windows,
        position: ((cx - output.x as i64) as f64, (cy - output.y as i64) as f64),
        selection: None,
        drag: None,
        result: None,
    };
    while !state.exit {
        queue.blocking_dispatch(&mut state)?;
        if state.dirty && !state.exit {
            state.dirty = false;
            state.draw(&qh);
        }
    }
    state.layer.wl_surface().attach(None, 0, 0);
    state.layer.commit();
    conn.flush()?;
    Ok(state.result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_drag_normalizes() {
        let r = drag_rect((400., 500.), (20., 30.));
        assert_eq!(r.raw, "20,30 380x470");
    }
}
