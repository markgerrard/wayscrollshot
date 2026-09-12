//! Shared antialiased typography and quiet, rounded surfaces for capture UI.
use ab_glyph::{point, Font, FontArc, ScaleFont};
use std::sync::OnceLock;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

fn font() -> &'static FontArc {
    static FONT: OnceLock<FontArc> = OnceLock::new();
    FONT.get_or_init(|| {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans"])
            .output()
            .expect("fontconfig required");
        let bytes = std::fs::read(String::from_utf8_lossy(&output.stdout).trim())
            .expect("sans font required");
        FontArc::try_from_vec(bytes).expect("valid font")
    })
}
pub fn text_width(text: &str, size: f32) -> f32 {
    let f = font().as_scaled(size);
    text.chars().map(|c| f.h_advance(f.glyph_id(c))).sum()
}
pub fn text(p: &mut Pixmap, label: &str, x: f32, y: f32, size: f32, color: [u8; 4]) {
    let f = font().as_scaled(size);
    let mut pen = x;
    for c in label.chars() {
        let mut glyph = f.scaled_glyph(c);
        glyph.position = point(pen, y + f.ascent());
        pen += f.h_advance(glyph.id);
        if let Some(outline) = font().outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            let w = p.width();
            let h = p.height();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    return;
                }
                let idx = ((py as u32 * w + px as u32) * 4) as usize;
                let a = coverage * color[3] as f32 / 255.;
                let dest = &mut p.data_mut()[idx..idx + 4];
                for i in 0..3 {
                    dest[i] = (color[i] as f32 * a + dest[i] as f32 * (1. - a)).round() as u8;
                }
                dest[3] = (255. * a + dest[3] as f32 * (1. - a)).round() as u8;
            });
        }
    }
}
pub fn rounded(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 4]) {
    if w <= 0. || h <= 0. {
        return;
    }
    let r = r.min(w / 2.).min(h / 2.);
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.quad_to(x + w, y, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.quad_to(x + w, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.quad_to(x, y + h, x, y + h - r);
    b.line_to(x, y + r);
    b.quad_to(x, y, x + r, y);
    b.close();
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
    p.fill_path(
        &b.finish().unwrap(),
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}
pub fn to_canvas(p: &Pixmap, canvas: &mut [u8], width: u32, y: u32) {
    let start = (width * y * 4) as usize;
    for (src, dst) in p
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(canvas[start..].as_chunks_mut::<4>().0.iter_mut())
    {
        dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
    }
}

/// Slice the generated sprite at runtime; the original artwork stays intact.
pub fn icon(p: &mut Pixmap, index: usize, x: f32, y: f32, size: u32) {
    static ICONS: OnceLock<Vec<image::RgbaImage>> = OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        let sheet = image::load_from_memory(include_bytes!("../assets/capture-icons.png"))
            .unwrap()
            .to_rgba8();
        // Bounding boxes of the six generated symbols, including their highlights.
        let mut icons: Vec<_> = [
            (120, 260, 240, 260),
            (515, 260, 240, 260),
            (925, 270, 215, 245),
            (125, 760, 215, 225),
            (525, 760, 215, 225),
            (905, 750, 250, 250),
        ]
        .iter()
        .map(|&(x, y, w, h)| {
            image::imageops::resize(
                &image::imageops::crop_imm(&sheet, x, y, w, h).to_image(),
                20,
                20,
                image::imageops::FilterType::Lanczos3,
            )
        })
        .collect();
        let modes = image::load_from_memory(include_bytes!("../assets/capture-modes.png"))
            .unwrap()
            .to_rgba8();
        for (x, y, w, h) in [(170, 160, 590, 590), (1080, 180, 530, 530)] {
            icons.push(image::imageops::resize(
                &image::imageops::crop_imm(&modes, x, y, w, h).to_image(),
                20,
                20,
                image::imageops::FilterType::Lanczos3,
            ));
        }
        icons
    });
    debug_assert_eq!(size, 20);
    let im = &icons[index];
    for (ix, iy, src) in im.enumerate_pixels() {
        let dx = x as i32 + ix as i32;
        let dy = y as i32 + iy as i32;
        if dx < 0 || dy < 0 || dx >= p.width() as i32 || dy >= p.height() as i32 {
            continue;
        }
        let off = ((dy as u32 * p.width() + dx as u32) * 4) as usize;
        let a = src[3] as f32 / 255.;
        let dst = &mut p.data_mut()[off..off + 4];
        for i in 0..3 {
            dst[i] = (src[i] as f32 * a + dst[i] as f32 * (1. - a)) as u8;
        }
        dst[3] = (src[3] as f32 + dst[3] as f32 * (1. - a)) as u8;
    }
}

/// Hairline edge and soft layered shadow keep floating panels distinct.
pub fn panel(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    for spread in (1..=5).rev() {
        let d = spread as f32;
        rounded(
            p,
            x - d,
            y - d + 3.,
            w + 2. * d,
            h + 2. * d,
            radius + d,
            [0, 0, 0, 8],
        );
    }
    rounded(p, x, y, w, h, radius, [85, 88, 100, 235]);
    rounded(
        p,
        x + 1.,
        y + 1.,
        w - 2.,
        h - 2.,
        radius - 1.,
        [28, 29, 35, 252],
    );
}
