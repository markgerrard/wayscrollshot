use anyhow::{Context, Result};
use smithay_client_toolkit::{
    delegate_output, delegate_registry,
    output::{OutputData, OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{
    globals::registry_queue_init, protocol::wl_output, Connection, Proxy, QueueHandle,
};

use crate::types::Region;

use super::PREVIEW_GAP;

/// Output geometry in global compositor coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputRect {
    pub(crate) id: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl OutputRect {
    fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }

    fn contains_point(self, x: i64, y: i64) -> bool {
        let left = i64::from(self.x);
        let top = i64::from(self.y);
        let right = i64::from(self.right());
        let bottom = i64::from(self.bottom());
        x >= left && x < right && y >= top && y < bottom
    }

    fn distance_squared_to_point(self, x: i64, y: i64) -> i128 {
        let left = i64::from(self.x);
        let top = i64::from(self.y);
        let right = i64::from(self.right());
        let bottom = i64::from(self.bottom());

        let dx = if x < left {
            left - x
        } else if x >= right {
            x - right + 1
        } else {
            0
        };

        let dy = if y < top {
            top - y
        } else if y >= bottom {
            y - bottom + 1
        } else {
            0
        };

        let dx = i128::from(dx);
        let dy = i128::from(dy);
        dx * dx + dy * dy
    }
}

fn i64_to_i32_saturating(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn output_rect_from_info(info: &smithay_client_toolkit::output::OutputInfo) -> Option<OutputRect> {
    let (fallback_width, fallback_height) = info
        .modes
        .iter()
        .find(|mode| mode.current)
        .or_else(|| info.modes.first())
        .map(|mode| mode.dimensions)
        .unwrap_or((0, 0));

    let (width, height) = info
        .logical_size
        .unwrap_or((fallback_width, fallback_height));
    if width <= 0 || height <= 0 {
        return None;
    }

    let (x, y) = info.logical_position.unwrap_or(info.location);

    Some(OutputRect {
        id: info.id,
        x,
        y,
        width,
        height,
    })
}

fn output_id(output: &wl_output::WlOutput) -> Option<u32> {
    output
        .data::<OutputData>()
        .map(|data| data.with_output_info(|info| info.id))
}

pub(crate) fn find_output_by_id(
    output_state: &OutputState,
    id: u32,
) -> Option<wl_output::WlOutput> {
    output_state
        .outputs()
        .find(|output| output_id(output) == Some(id))
}

pub(crate) fn select_output_for_region(
    region: &Region,
    outputs: &[OutputRect],
) -> Option<OutputRect> {
    if outputs.is_empty() {
        return None;
    }

    let center_x = i64::from(region.x).saturating_add(i64::from(region.w) / 2);
    let center_y = i64::from(region.y).saturating_add(i64::from(region.h) / 2);

    if let Some(output) = outputs
        .iter()
        .copied()
        .find(|output| output.contains_point(center_x, center_y))
    {
        return Some(output);
    }

    outputs
        .iter()
        .copied()
        .min_by_key(|output| output.distance_squared_to_point(center_x, center_y))
}

fn compute_preview_margin_left(region: &Region, preview_width: u32, outputs: &[OutputRect]) -> i32 {
    let preview_width = preview_width.max(1);
    let preview_width_i64 = i64::from(preview_width);
    let right_candidate = i64::from(region.x)
        .saturating_add(i64::from(region.w))
        .saturating_add(i64::from(PREVIEW_GAP));

    let Some(output) = select_output_for_region(region, outputs) else {
        return i64_to_i32_saturating(right_candidate);
    };

    let output_left = i64::from(output.x);
    let output_right = i64::from(output.right());

    if right_candidate.saturating_add(preview_width_i64) <= output_right {
        return i64_to_i32_saturating(right_candidate);
    }

    let left_candidate = i64::from(region.x)
        .saturating_sub(i64::from(PREVIEW_GAP))
        .saturating_sub(preview_width_i64);

    if left_candidate >= output_left {
        return i64_to_i32_saturating(left_candidate);
    }

    let max_left = output_right.saturating_sub(preview_width_i64);
    let clamped = if max_left < output_left {
        output_left
    } else {
        right_candidate.clamp(output_left, max_left)
    };
    i64_to_i32_saturating(clamped)
}

/// Computes layer-shell margins using the selected output as coordinate origin.
pub(crate) fn compute_layer_margins(
    region: &Region,
    preview_width: u32,
    outputs: &[OutputRect],
) -> (i32, i32) {
    let left_global = compute_preview_margin_left(region, preview_width, outputs);
    if let Some(output) = select_output_for_region(region, outputs) {
        (
            region.y.saturating_sub(output.y),
            left_global.saturating_sub(output.x),
        )
    } else {
        (region.y, left_global)
    }
}

pub(crate) fn output_rects_from_state(output_state: &OutputState) -> Vec<OutputRect> {
    output_state
        .outputs()
        .filter_map(|output| output_state.info(&output))
        .filter_map(|info| output_rect_from_info(&info))
        .collect()
}

struct OutputProbe {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for OutputProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

delegate_output!(OutputProbe);
delegate_registry!(OutputProbe);

impl ProvidesRegistryState for OutputProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState);
}

/// Probes output geometry from a fresh Wayland connection.
pub(crate) fn probe_output_rects() -> Result<Vec<OutputRect>> {
    let conn =
        Connection::connect_to_env().context("failed to connect to Wayland for output probe")?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).context("failed to init Wayland registry for output probe")?;
    let qh = event_queue.handle();

    let mut output_probe = OutputProbe {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
    };

    event_queue
        .roundtrip(&mut output_probe)
        .context("failed to gather output geometry")?;

    Ok(output_rects_from_state(&output_probe.output_state))
}

#[cfg(test)]
mod tests {
    use super::{compute_preview_margin_left, OutputRect, PREVIEW_GAP};
    use crate::types::Region;

    fn region(x: i32, y: i32, w: u32, h: u32) -> Region {
        Region {
            raw: String::new(),
            x,
            y,
            w,
            h,
        }
    }

    fn output(x: i32, y: i32, width: i32, height: i32) -> OutputRect {
        OutputRect {
            id: 0,
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn places_preview_to_right_when_space_available() {
        let outputs = [output(0, 0, 1920, 1080)];
        let region = region(200, 100, 400, 300);

        let left = compute_preview_margin_left(&region, 280, &outputs);

        assert_eq!(left, 200 + 400 + PREVIEW_GAP);
    }

    #[test]
    fn moves_preview_to_left_when_right_side_overflows() {
        let outputs = [output(0, 0, 1920, 1080)];
        let region = region(1700, 100, 200, 300);

        let left = compute_preview_margin_left(&region, 280, &outputs);

        assert_eq!(left, 1700 - PREVIEW_GAP - 280);
    }

    #[test]
    fn clamps_preview_when_neither_side_fits() {
        let outputs = [output(0, 0, 800, 600)];
        let region = region(300, 120, 200, 250);

        let left = compute_preview_margin_left(&region, 700, &outputs);

        assert_eq!(left, 100);
    }

    #[test]
    fn chooses_output_containing_region_center() {
        let outputs = [output(0, 0, 1920, 1080), output(1920, 0, 1920, 1080)];
        let region = region(3600, 100, 200, 300);

        let left = compute_preview_margin_left(&region, 400, &outputs);

        assert_eq!(left, 3600 - PREVIEW_GAP - 400);
    }

    #[test]
    fn falls_back_to_right_side_without_outputs() {
        let region = region(50, 60, 120, 220);

        let left = compute_preview_margin_left(&region, 300, &[]);

        assert_eq!(left, 50 + 120 + PREVIEW_GAP);
    }
}

/// Reserve a preview rectangle wholly outside the capture. Never clamp into it.
pub(crate) fn live_preview_area(
    region: &Region,
    desired_width: u32,
    outputs: &[OutputRect],
) -> Option<Region> {
    let o = select_output_for_region(region, outputs)?;
    let gap = 8;
    let left = o.x + gap;
    let right = o.right() - gap;
    let top = o.y + gap;
    let bottom = o.bottom() - gap;
    let rx = region.x;
    let ry = region.y;
    let rr = rx + region.w as i32;
    let rb = ry + region.h as i32;
    let side_slots = [
        (rr + gap, top, right - rr - gap, bottom - top),
        (left, top, rx - gap - left, bottom - top),
    ];
    if let Some((x, y, w, h)) = side_slots
        .into_iter()
        .find(|(_, _, w, h)| *w >= 120 && *h >= (super::INITIAL_HEIGHT + 80) as i32)
    {
        return Some(Region {
            raw: String::new(),
            x,
            y,
            w: (w as u32).min(desired_width),
            h: h as u32,
        });
    }
    let fallback_slots = [
        (left, top, right - left, ry - gap - top),
        (left, rb + gap, right - left, bottom - rb - gap),
    ];
    fallback_slots
        .into_iter()
        .filter(|(_, _, w, h)| *w >= 96 && *h >= (super::INITIAL_HEIGHT + 32) as i32)
        .max_by_key(|(_, _, w, h)| (*w).min(desired_width as i32) * (*h).min(480))
        .map(|(x, y, w, h)| Region {
            raw: String::new(),
            x,
            y,
            w: (w as u32).min(desired_width),
            h: h as u32,
        })
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    fn wide_region_gets_preview_above_without_overlap() {
        let r = Region {
            raw: String::new(),
            x: 20,
            y: 180,
            w: 1880,
            h: 850,
        };
        let p = live_preview_area(
            &r,
            280,
            &[OutputRect {
                id: 0,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            }],
        )
        .unwrap();
        assert!(p.y + p.h as i32 <= r.y);
    }
    #[test]
    fn narrow_strip_cannot_fit_header_controls_and_preview() {
        let r = Region {
            raw: String::new(),
            x: 0,
            y: 116,
            w: 1920,
            h: 964,
        };
        let output = OutputRect {
            id: 0,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!(live_preview_area(&r, 280, &[output]).is_none());
    }
    #[test]
    fn fullscreen_has_no_safe_live_preview() {
        let r = Region {
            raw: String::new(),
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        assert!(live_preview_area(
            &r,
            280,
            &[OutputRect {
                id: 0,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }]
        )
        .is_none());
    }

    #[test]
    fn side_preview_uses_the_full_available_height() {
        let r = Region {
            raw: String::new(),
            x: 20,
            y: 100,
            w: 1300,
            h: 800,
        };
        let p = live_preview_area(
            &r,
            280,
            &[OutputRect {
                id: 0,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            }],
        )
        .unwrap();
        assert_eq!(p.x, 1328);
        assert_eq!(p.h, 1064);
    }
}
