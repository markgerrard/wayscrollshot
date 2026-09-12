use tiny_skia::Pixmap;

use super::OverlayMode;
use crate::constants::CONTROL_BAR_HEIGHT;
use crate::types::PreviewImage;

/// Rounded controls with generated icon artwork and real font labels.
pub(super) fn draw_control_bar(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    bar_y: u32,
    paused: bool,
    auto_enabled: bool,
    hover: Option<u32>,
    mode: OverlayMode,
) {
    let h = CONTROL_BAR_HEIGHT.min(height.saturating_sub(bar_y));
    let Some(mut p) = Pixmap::new(width, h) else {
        return;
    };
    crate::ui::rounded(
        &mut p,
        0.,
        0.,
        width as f32,
        h as f32,
        12.,
        [27, 28, 33, 250],
    );
    let count = 4;
    let labels = match mode {
        OverlayMode::Review => vec!["Save", "Copy", "Cloud", "Cancel"],
        OverlayMode::Gallery => vec!["Back", "Next", "Open", "Close"],
        OverlayMode::Live => vec![
            "Done",
            "Copy",
            if !auto_enabled {
                "Auto"
            } else if paused {
                "Resume"
            } else {
                "Pause"
            },
            "Cancel",
        ],
    };
    let icons = match mode {
        OverlayMode::Review => vec![0, 1, 5, 4],
        OverlayMode::Gallery => vec![3, 2, 5, 4],
        OverlayMode::Live => vec![
            0,
            1,
            if !auto_enabled {
                7
            } else if paused {
                3
            } else {
                2
            },
            4,
        ],
    };
    let segment = width as f32 / count as f32;
    for i in 0..count {
        let x = i as f32 * segment;
        crate::ui::rounded(
            &mut p,
            x + 3.,
            4.,
            segment - 6.,
            h as f32 - 8.,
            9.,
            if hover == Some(i as u32) {
                [77, 79, 89, 255]
            } else if i == 0 {
                [65, 76, 96, 255]
            } else {
                [42, 43, 50, 255]
            },
        );
        crate::ui::icon(&mut p, icons[i], x + segment / 2. - 10., 10., 20);
        if width >= 180 {
            let label = labels[i];
            crate::ui::text(
                &mut p,
                label,
                x + (segment - crate::ui::text_width(label, 13.)) / 2.,
                36.,
                13.,
                [243, 243, 246, 255],
            );
        }
    }
    crate::ui::to_canvas(&p, canvas, width, bar_y);
}

pub(super) fn blit_preview_bottom(
    canvas: &mut [u8],
    canvas_width: u32,
    available_height: u32,
    preview: &PreviewImage,
) {
    let bytes_per_row = canvas_width.saturating_mul(4) as usize;
    if bytes_per_row == 0 || available_height == 0 {
        return;
    }

    let max_cols = preview.width.min(canvas_width);

    let (src_start_y, display_height) = if preview.height <= available_height {
        (0, preview.height)
    } else {
        (preview.height - available_height, available_height)
    };

    for y in 0..display_height {
        let src_y = src_start_y + y;
        let src_row = (src_y * preview.width * 4) as usize;
        let dst_row = (y * canvas_width * 4) as usize;
        for x in 0..max_cols {
            let src_idx = src_row + (x * 4) as usize;
            let dst_idx = dst_row + (x * 4) as usize;
            if src_idx + 3 < preview.pixels.len() && dst_idx + 3 < canvas.len() {
                canvas[dst_idx] = preview.pixels[src_idx + 2];
                canvas[dst_idx + 1] = preview.pixels[src_idx + 1];
                canvas[dst_idx + 2] = preview.pixels[src_idx];
                canvas[dst_idx + 3] = preview.pixels[src_idx + 3];
            }
        }
    }
}

#[allow(dead_code)]
fn blit_preview(canvas: &mut [u8], canvas_width: u32, preview: &PreviewImage, offset_y: u32) {
    let bytes_per_row = canvas_width.saturating_mul(4) as usize;
    if bytes_per_row == 0 {
        return;
    }
    let canvas_height = (canvas.len() / bytes_per_row) as u32;
    if offset_y >= canvas_height {
        return;
    }

    let max_rows = (canvas_height - offset_y).min(preview.height);
    let max_cols = preview.width.min(canvas_width);

    for y in 0..max_rows {
        let src_row = (y * preview.width * 4) as usize;
        let dst_row = ((offset_y + y) * canvas_width * 4) as usize;
        for x in 0..max_cols {
            let src_idx = src_row + (x * 4) as usize;
            let dst_idx = dst_row + (x * 4) as usize;
            canvas[dst_idx] = preview.pixels[src_idx + 2];
            canvas[dst_idx + 1] = preview.pixels[src_idx + 1];
            canvas[dst_idx + 2] = preview.pixels[src_idx];
            canvas[dst_idx + 3] = preview.pixels[src_idx + 3];
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn capped_preview_displays_latest_rows() {
        let preview = PreviewImage {
            width: 1,
            height: 5,
            pixels: (0..5).flat_map(|n| [n, n, n, 255]).collect(),
        };
        let mut canvas = vec![0; 12];
        blit_preview_bottom(&mut canvas, 1, 3, &preview);
        assert_eq!(canvas, vec![2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]);
    }
}
