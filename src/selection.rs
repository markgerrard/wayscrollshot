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

const TOOLBAR_Y: f32 = 64.;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Area,
    Screen,
    Window,
    Scroll,
}

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
    hover: Option<usize>,
    mode: Mode,
    show_modes: bool,
    window_locked: bool,
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
            if self.valid_selection() {
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
    fn valid_selection(&self) -> bool {
        self.selection.as_ref().is_some_and(|r| {
            if self.mode == Mode::Scroll {
                r.w >= 32 && r.h >= 160
            } else {
                r.w > 0 && r.h > 0
            }
        })
    }
    fn toolbar(&self) -> Vec<(f64, f64, &'static str, usize)> {
        if self.show_modes {
            vec![
                (0., 94., "Area", 0),
                (98., 94., "Full screen", 5),
                (196., 94., "Window", 6),
                (294., 94., "Scrolling", 7),
                (408., 90., "Cancel", 4),
                (504., 124., "Capture", 0),
            ]
        } else {
            vec![
                (0., 106., "Cancel", 4),
                (110., 166., "Full screen", 5),
                (280., 160., "Capture", 0),
            ]
        }
    }
    fn toolbar_width(&self) -> f64 {
        if self.show_modes {
            628.
        } else {
            440.
        }
    }
    fn window_at_pointer(&mut self) {
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
    fn toolbar_hover(&self) -> Option<usize> {
        let (x, y) = self.position;
        let bx = (self.width as f64 - self.toolbar_width()) / 2.;
        if y < TOOLBAR_Y as f64 - 4. || y > TOOLBAR_Y as f64 + 52. {
            return None;
        }
        self.toolbar()
            .iter()
            .position(|(offset, w, _, _)| x >= bx + offset && x < bx + offset + w)
    }
    fn press(&mut self) {
        if let Some(button) = self.toolbar_hover() {
            if self.show_modes {
                match button {
                    0 => {
                        self.mode = Mode::Area;
                        self.selection = None;
                    }
                    1 => {
                        self.mode = Mode::Screen;
                        self.choose_screen();
                    }
                    2 => {
                        self.window_locked = false;
                        self.mode = Mode::Window;
                        self.selection = None;
                    }
                    3 => {
                        self.mode = Mode::Scroll;
                        self.selection = None;
                    }
                    4 => self.exit = true,
                    _ => self.finish(),
                }
            } else {
                match button {
                    0 => self.exit = true,
                    1 => self.choose_screen(),
                    _ => self.finish(),
                }
            }
            return;
        }
        if self.mode == Mode::Window && !self.window_locked {
            self.window_at_pointer();
            self.window_locked = true;
            return;
        }
        let (x, y) = self.position;
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
                ui::rounded(&mut p, a - 5., b - 5., 10., 10., 3., [25, 29, 40, 240]);
                ui::rounded(&mut p, a - 3., b - 3., 6., 6., 2., [255, 255, 255, 255]);
            }
            let label = format!("{} × {}", r.w, r.h);
            let tw = ui::text_width(&label, 15.);
            let lx = (x + w - tw - 36.).clamp(4., (self.width as f32 - tw - 28.).max(4.));
            let mut ly = if y > 48. {
                y - 40.
            } else {
                (y + 12.).min(self.height as f32 - 40.)
            };
            let bar_left = (self.width as f32 - self.toolbar_width() as f32) / 2. - 8.;
            let bar_right = self.width as f32 - bar_left;
            if lx < bar_right
                && lx + tw + 24. > bar_left
                && ly < TOOLBAR_Y + 84.
                && ly + 28. > TOOLBAR_Y - 12.
            {
                ly = TOOLBAR_Y + 96.;
            }
            ui::rounded(&mut p, lx, ly, tw + 24., 28., 8., [28, 29, 34, 245]);
            ui::text(&mut p, &label, lx + 12., ly + 5., 15., [255, 255, 255, 255]);
        }
        let valid = self.valid_selection();
        let hint = if self.selection.is_some() && !valid {
            if self.mode == Mode::Scroll {
                "Select at least 32 × 160 pixels"
            } else {
                "Drag to select an area"
            }
        } else if self.selection.is_some() {
            "Drag edges to adjust · Enter to capture · Esc to cancel"
        } else if self.mode == Mode::Window {
            "Point at a window · Enter to capture · Esc to cancel"
        } else {
            "Drag to select · Space for window · Enter to capture · Esc to cancel"
        };
        let tw = self.toolbar_width() as f32;
        let bx = (self.width as f32 - tw) / 2.;
        let by = TOOLBAR_Y;
        ui::panel(&mut p, bx - 8., by - 12., tw + 16., 96., 18.);
        for (i, (offset, w, label, icon)) in self.toolbar().iter().enumerate() {
            let (offset, w) = (*offset as f32, *w as f32);
            let primary = *label == "Capture";
            let selected = self.show_modes
                && i < 4
                && i == match self.mode {
                    Mode::Area => 0,
                    Mode::Screen => 1,
                    Mode::Window => 2,
                    Mode::Scroll => 3,
                };
            ui::rounded(
                &mut p,
                bx + offset,
                by - 4.,
                w,
                56.,
                10.,
                if primary && !valid {
                    [38, 40, 48, 255]
                } else if self.hover == Some(i) {
                    [69, 77, 96, 255]
                } else if primary {
                    [64, 106, 179, 255]
                } else if selected {
                    [60, 64, 77, 255]
                } else {
                    [33, 35, 42, 255]
                },
            );
            ui::icon(&mut p, *icon, bx + offset + w / 2. - 10., by + 2., 20);
            ui::text(
                &mut p,
                label,
                bx + offset + (w - ui::text_width(label, 14.)) / 2.,
                by + 30.,
                14.,
                if primary && !valid {
                    [121, 126, 142, 255]
                } else {
                    [240, 241, 246, 255]
                },
            );
        }
        ui::text(
            &mut p,
            hint,
            (self.width as f32 - ui::text_width(hint, 13.)) / 2.,
            by + 65.,
            13.,
            [167, 174, 190, 255],
        );
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
            Keysym::space => {
                if self.show_modes {
                    self.mode = Mode::Window;
                }
                self.choose_window();
                self.window_locked = true;
            }
            Keysym::f | Keysym::F => {
                if self.show_modes {
                    self.mode = Mode::Screen;
                }
                self.choose_screen();
            }
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
            let hover = self.toolbar_hover();
            if self.mode == Mode::Window
                && !self.window_locked
                && hover.is_none()
                && self.drag.is_none()
                && matches!(event.kind, PointerEventKind::Motion { .. })
            {
                self.window_at_pointer();
                self.dirty = true;
            }
            if hover != self.hover {
                self.hover = hover;
                self.dirty = true;
            }
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
pub fn select(scrolling: bool, show_modes: bool, initial: &str) -> Result<Option<(Region, bool)>> {
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
        hover: None,
        mode: if scrolling { Mode::Scroll } else { Mode::Area },
        show_modes,
        window_locked: false,
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
    match initial {
        "screen" => {
            state.choose_screen();
            if !scrolling {
                state.mode = Mode::Screen;
            }
        }
        "window" => {
            state.mode = Mode::Window;
        }
        _ => {}
    }
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
    Ok(state.result.map(|r| (r, state.mode == Mode::Scroll)))
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
