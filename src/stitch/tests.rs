use image::{imageops, Rgba};

use super::*;

/// 构造包含多种纹理的长截图测试画布
///
/// `width` 和 `height` 为画布尺寸；返回用于 ORB 匹配的测试图像
fn make_scroll_canvas(width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([245, 245, 245, 255]));

    for y in (0..height).step_by(36) {
        let accent = ((y / 3) % 180) as u8;
        for x in 24..width.saturating_sub(24) {
            let stripe = if (x / 7 + y / 11) % 2 == 0 { 220 } else { 180 };
            img.put_pixel(x, y, Rgba([accent, stripe, 80, 255]));
            if y + 1 < height {
                img.put_pixel(x, y + 1, Rgba([30, 30, 30, 255]));
            }
        }
    }

    for block in 0..10 {
        let y0 = 30 + block * 80;
        let block_h = 34 + (block % 3) * 8;
        let color = [
            ((40u16 + block as u16 * 17) % 200) as u8,
            ((90u16 + block as u16 * 11) % 200) as u8,
            ((140u16 + block as u16 * 19) % 200) as u8,
            255,
        ];
        for y in y0..(y0 + block_h).min(height) {
            for x in 30..width.saturating_sub(30) {
                if x % (9 + block % 5) == 0 || y % (7 + block % 4) == 0 {
                    img.put_pixel(x, y, Rgba(color));
                }
            }
        }
    }

    for col in [42, 96, 154, 211, 268] {
        if col >= width {
            continue;
        }
        for y in 20..height.saturating_sub(20) {
            if (y / 13) % 3 != 0 {
                img.put_pixel(col, y, Rgba([20, 20, 20, 255]));
            }
        }
    }

    img
}

/// 从长画布裁剪一帧测试截图
///
/// `canvas` 为长画布，`y` 为起始行，`height` 为帧高度；返回裁剪后的截图
fn crop_frame(canvas: &RgbaImage, y: u32, height: u32) -> RgbaImage {
    imageops::crop_imm(canvas, 0, y, canvas.width(), height).to_image()
}

/// 构造低特征密度的横线测试画布
///
/// `width` 和 `height` 为画布尺寸；返回用于模板回退测试的图像
fn make_line_canvas(width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([250, 250, 250, 255]));
    let mut y = 16u32;
    let mut band = 0u32;

    while y < height.saturating_sub(16) {
        let band_h = 6 + (band % 5) * 4;
        let gray = (40 + ((band * 29) % 160)) as u8;
        for yy in y..(y + band_h).min(height) {
            for x in 28..width.saturating_sub(28) {
                img.put_pixel(x, yy, Rgba([gray, gray, gray, 255]));
            }
        }
        y += band_h + 9 + (band % 4) * 3;
        band += 1;
    }

    img
}

/// 验证 ORB 能估算标准垂直滚动偏移；该测试无传参和返回值
#[test]
fn opencv_orb_estimates_vertical_offset() {
    let canvas = make_scroll_canvas(320, 1000);
    let first = crop_frame(&canvas, 0, 320);
    let second = crop_frame(&canvas, 84, 320);

    let estimate = estimate_orb_offset(&first, &second, 120)
        .expect("opencv estimate")
        .expect("orb match");

    assert!((estimate.dy - 84.0).abs() <= 4.0, "dy={}", estimate.dy);
    assert!(
        estimate.confidence < 3.5,
        "confidence={}",
        estimate.confidence
    );
}

/// 验证坏帧不会替换 ORB 锚点；该测试无传参和返回值
#[test]
fn opencv_orb_keeps_anchor_after_bad_frame() {
    let canvas = make_scroll_canvas(320, 1000);
    let first = crop_frame(&canvas, 0, 320);
    let shifted = crop_frame(&canvas, 96, 320);
    let bad = RgbaImage::from_pixel(320, 320, Rgba([255, 255, 255, 255]));

    let mut stitcher = Stitcher::new(MatchConfig {
        min_overlap: 120,
        accept_diff: 3.5,
        min_append: 10,
        approx_diff: 1.0,
        algorithm: Algorithm::OpenCvOrb,
        match_width: 320,
    });

    assert!(matches!(
        stitcher.push_frame(first),
        StitchOutcome::FirstFrame
    ));
    assert!(matches!(stitcher.push_frame(bad), StitchOutcome::NoMatch));

    match stitcher.push_frame(shifted) {
        StitchOutcome::Appended { added } => assert!((92..=100).contains(&added), "{added}"),
        _ => panic!("expected appended after bad frame"),
    }
}

/// 验证低特征截图会回退到模板匹配；该测试无传参和返回值
#[test]
fn opencv_orb_falls_back_to_template_on_low_feature_frames() {
    let canvas = make_line_canvas(320, 1000);
    let first = crop_frame(&canvas, 0, 320);
    let second = crop_frame(&canvas, 72, 320);

    let mut stitcher = Stitcher::new(MatchConfig {
        min_overlap: 120,
        accept_diff: 3.5,
        min_append: 10,
        approx_diff: 1.0,
        algorithm: Algorithm::OpenCvOrb,
        match_width: 320,
    });

    assert!(matches!(
        stitcher.push_frame(first),
        StitchOutcome::FirstFrame
    ));

    match stitcher.push_frame(second) {
        StitchOutcome::Appended { added } => assert!((68..=76).contains(&added), "{added}"),
        _ => panic!("expected appended via template fallback"),
    }
}

/// 验证降低最小重叠高度后可以处理较大滚动跨度；该测试无传参和返回值
#[test]
fn opencv_orb_relaxed_overlap_handles_large_jump() {
    let canvas = make_scroll_canvas(320, 1200);
    let first = crop_frame(&canvas, 0, 320);
    let second = crop_frame(&canvas, 208, 320);

    let mut stitcher = Stitcher::new(MatchConfig {
        min_overlap: 120,
        accept_diff: 3.5,
        min_append: 10,
        approx_diff: 1.0,
        algorithm: Algorithm::OpenCvOrb,
        match_width: 320,
    });

    assert!(matches!(
        stitcher.push_frame(first),
        StitchOutcome::FirstFrame
    ));

    match stitcher.push_frame(second) {
        StitchOutcome::Appended { added } => assert!((202..=214).contains(&added), "{added}"),
        _ => panic!("expected appended via relaxed overlap"),
    }
}

fn column_stitcher() -> Stitcher {
    Stitcher::new(MatchConfig {
        min_overlap: 100,
        accept_diff: 3.5,
        min_append: 2,
        approx_diff: 0.5,
        algorithm: Algorithm::ColSample,
        match_width: 280,
    })
}

#[test]
fn columns_preserve_anchor_after_rejected_frame() {
    let canvas = make_scroll_canvas(320, 1000);
    let mut s = column_stitcher();
    s.push_frame(crop_frame(&canvas, 0, 320));
    let bad = RgbaImage::from_pixel(320, 320, Rgba([0, 0, 0, 255]));
    assert!(matches!(s.push_frame(bad), StitchOutcome::NoMatch));
    assert!(matches!(
        s.push_frame(crop_frame(&canvas, 84, 320)),
        StitchOutcome::Appended { added: 84 }
    ));
    assert_eq!(
        s.full_image.as_ref().unwrap().as_ref(),
        &crop_frame(&canvas, 0, 404)
    );
}

#[test]
fn columns_preserve_anchor_during_reverse_scroll() {
    let canvas = make_scroll_canvas(320, 1000);
    let mut s = column_stitcher();
    s.push_frame(crop_frame(&canvas, 0, 320));
    s.push_frame(crop_frame(&canvas, 84, 320));
    s.push_frame(crop_frame(&canvas, 40, 320));
    s.push_frame(crop_frame(&canvas, 168, 320));
    assert_eq!(
        s.full_image.as_ref().unwrap().as_ref(),
        &crop_frame(&canvas, 0, 488)
    );
}

#[test]
fn pixel_verification_rejects_equal_average_different_content() {
    let mut a = RgbaImage::new(320, 320);
    let mut b = a.clone();
    for y in 0..320 {
        for x in 0..320 {
            let value = if x % 2 == 0 { 0 } else { 255 };
            a.put_pixel(x, y, Rgba([value, value, value, 255]));
            b.put_pixel(x, y, Rgba([255 - value, 255 - value, 255 - value, 255]));
        }
    }
    assert!(!verified_overlap(&a, &b, 40));
}

#[test]
fn blank_overlap_is_not_evidence_of_scroll() {
    let frame = RgbaImage::from_pixel(320, 320, Rgba([255, 255, 255, 255]));
    assert!(!verified_overlap(&frame, &frame, 80));
}

#[test]
fn fixed_header_and_footer_are_not_repeated_at_joins() {
    let canvas = make_scroll_canvas(320, 1000);
    let decorate = |mut frame: RgbaImage| {
        let h = frame.height();
        for y in 0..32 {
            for x in 0..frame.width() {
                frame.put_pixel(x, y, Rgba([20, 30, 40, 255]));
                frame.put_pixel(x, h - 1 - y, Rgba([40, 30, 20, 255]));
            }
        }
        frame
    };
    let mut s = column_stitcher();
    s.push_frame(decorate(crop_frame(&canvas, 0, 320)));
    assert!(matches!(
        s.push_frame(decorate(crop_frame(&canvas, 84, 320))),
        StitchOutcome::Appended { added: 84 }
    ));
    assert!(matches!(
        s.push_frame(decorate(crop_frame(&canvas, 168, 320))),
        StitchOutcome::Appended { added: 84 }
    ));
    assert_eq!(
        s.full_image.as_ref().unwrap().as_ref(),
        &decorate(crop_frame(&canvas, 0, 488))
    );
}

#[test]
fn large_crop_relative_steps_preserve_every_row() {
    let canvas = make_scroll_canvas(480, 2600);
    let mut s = Stitcher::new(MatchConfig {
        min_overlap: 73,
        accept_diff: 3.5,
        min_append: 2,
        approx_diff: 0.5,
        algorithm: Algorithm::ColSample,
        match_width: 280,
    });
    s.push_frame(crop_frame(&canvas, 0, 880));
    for y in [480, 960, 1440] {
        assert!(matches!(
            s.push_frame(crop_frame(&canvas, y, 880)),
            StitchOutcome::Appended { added: 480 }
        ));
    }
    assert_eq!(
        s.full_image.as_ref().unwrap().as_ref(),
        &crop_frame(&canvas, 0, 2320)
    );
}

#[test]
fn rejected_large_jump_can_recover_by_scrolling_partway_back() {
    let canvas = make_scroll_canvas(480, 2600);
    let mut s = Stitcher::new(MatchConfig {
        min_overlap: 73,
        accept_diff: 3.5,
        min_append: 2,
        approx_diff: 0.5,
        algorithm: Algorithm::ColSample,
        match_width: 280,
    });
    s.push_frame(crop_frame(&canvas, 0, 880));
    assert!(matches!(
        s.push_frame(crop_frame(&canvas, 600, 880)),
        StitchOutcome::NoMatch
    ));
    assert!(matches!(
        s.push_frame(crop_frame(&canvas, 300, 880)),
        StitchOutcome::Appended { added: 300 }
    ));
    assert_eq!(
        s.full_image.as_ref().unwrap().as_ref(),
        &crop_frame(&canvas, 0, 1180)
    );
}
