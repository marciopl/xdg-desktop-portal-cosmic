#![allow(dead_code, unused_variables)]

use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::annotation::model::{AnnotationScene, LocalRect};

use std::cell::RefCell;
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache};

thread_local! {
    static FONT_SYSTEM: RefCell<FontSystem> = RefCell::new(FontSystem::new());
    static SWASH_CACHE: RefCell<SwashCache> = RefCell::new(SwashCache::new());
}

/// Convert straight-RGBA RgbaImage → premultiplied tiny-skia Pixmap.
pub fn pixmap_from_rgba(img: &RgbaImage) -> Pixmap {
    let mut pix = Pixmap::new(img.width(), img.height())
        .expect("non-zero pixmap dimensions");
    let dst = pix.data_mut();
    for (chunk, src) in dst.chunks_exact_mut(4).zip(img.chunks_exact(4)) {
        let r = src[0]; let g = src[1]; let b = src[2]; let a = src[3];
        let af = a as u32;
        chunk[0] = ((r as u32 * af + 127) / 255) as u8;
        chunk[1] = ((g as u32 * af + 127) / 255) as u8;
        chunk[2] = ((b as u32 * af + 127) / 255) as u8;
        chunk[3] = a;
    }
    pix
}

/// Convert premultiplied tiny-skia Pixmap → straight-RGBA RgbaImage.
pub fn rgba_from_pixmap(pix: &Pixmap) -> RgbaImage {
    let mut out = RgbaImage::new(pix.width(), pix.height());
    for (chunk, src) in out.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
        let r = src[0]; let g = src[1]; let b = src[2]; let a = src[3];
        if a == 0 {
            chunk[0] = 0; chunk[1] = 0; chunk[2] = 0; chunk[3] = 0;
        } else {
            chunk[0] = ((r as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            chunk[1] = ((g as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            chunk[2] = ((b as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            chunk[3] = a;
        }
    }
    out
}

pub fn render_annotations(target: &mut Pixmap, source: &Pixmap, scene: &AnnotationScene) {
    render_committed(target, source, scene);
    render_overlay_top(target, source, scene);
}

/// Render only committed annotations onto `target`. No in-progress, no crop dim.
/// Used by the widget to cache the static portion of the overlay between strokes.
pub fn render_committed(target: &mut Pixmap, source: &Pixmap, scene: &AnnotationScene) {
    for ann in scene.iter_committed() {
        render_one(target, source, ann);
    }
}

/// Render the dynamic overlay layer: in-progress annotation followed by the crop dim.
/// Drawn on top of `render_committed`'s output to produce the live preview each frame.
pub fn render_overlay_top(target: &mut Pixmap, source: &Pixmap, scene: &AnnotationScene) {
    if let Some(ann) = scene.in_progress() {
        render_one(target, source, ann);
    }
    if let Some(crop) = scene.crop() {
        render_crop_dim(target, crop);
    }
}

fn render_crop_dim(target: &mut Pixmap, crop: &LocalRect) {
    use tiny_skia::{FillRule, Paint, PathBuilder, Transform};
    let w = target.width() as f32;
    let h = target.height() as f32;
    // Even-odd path: outer rect = whole canvas, inner rect = crop. Filling with even-odd
    // produces a frame — everything except the crop interior.
    let outer = match tiny_skia::Rect::from_xywh(0.0, 0.0, w, h) {
        Some(r) => r,
        None => return,
    };
    let inner = match tiny_skia::Rect::from_xywh(
        crop.origin.x,
        crop.origin.y,
        crop.size.w.max(0.0),
        crop.size.h.max(0.0),
    ) {
        Some(r) => r,
        None => return,
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(outer);
    pb.push_rect(inner);
    let Some(path) = pb.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 128);
    target.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
}

fn render_one(target: &mut Pixmap, source: &Pixmap, ann: &crate::annotation::model::Annotation) {
    use tiny_skia::{PathBuilder, Stroke as TsStroke, Transform};
    use crate::annotation::model::Annotation;

    match ann {
        Annotation::Pen { points, stroke } => {
            if points.len() < 2 { return; }
            let mut pb = PathBuilder::new();
            pb.move_to(points[0].x, points[0].y);
            for p in &points[1..] { pb.line_to(p.x, p.y); }
            let Some(path) = pb.finish() else { return };
            let paint = make_paint(stroke.color);
            let mut s = TsStroke::default();
            s.width = stroke.width.max(0.5);
            s.line_cap = tiny_skia::LineCap::Round;
            s.line_join = tiny_skia::LineJoin::Round;
            target.stroke_path(&path, &paint, &s, Transform::identity(), None);
        }
        Annotation::Rectangle { rect, stroke } => {
            if rect.is_degenerate() { return; }
            let r = match tiny_skia::Rect::from_xywh(rect.origin.x, rect.origin.y, rect.size.w, rect.size.h) {
                Some(r) => r,
                None => return,
            };
            let path = PathBuilder::from_rect(r);
            let paint = make_paint(stroke.color);
            let mut s = TsStroke::default();
            s.width = stroke.width.max(0.5);
            target.stroke_path(&path, &paint, &s, Transform::identity(), None);
        }
        Annotation::Line { from, to, stroke } => {
            let mut pb = PathBuilder::new();
            pb.move_to(from.x, from.y);
            pb.line_to(to.x, to.y);
            let Some(path) = pb.finish() else { return };
            let paint = make_paint(stroke.color);
            let mut s = TsStroke::default();
            s.width = stroke.width.max(0.5);
            s.line_cap = tiny_skia::LineCap::Round;
            target.stroke_path(&path, &paint, &s, Transform::identity(), None);
        }
        Annotation::Arrow { from, to, stroke } => {
            // Shaft
            let mut pb = PathBuilder::new();
            pb.move_to(from.x, from.y);
            pb.line_to(to.x, to.y);
            let Some(path) = pb.finish() else { return };
            let paint = make_paint(stroke.color);
            let mut s = TsStroke::default();
            s.width = stroke.width.max(0.5);
            s.line_cap = tiny_skia::LineCap::Round;
            target.stroke_path(&path, &paint, &s, Transform::identity(), None);

            // Arrowhead — filled triangle at `to`, sized proportional to stroke width.
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.5 { return; }
            let head_len = (stroke.width * 4.0).max(8.0);
            let head_w = (stroke.width * 3.0).max(6.0);
            let ux = dx / len; let uy = dy / len;
            let bx = to.x - ux * head_len;
            let by = to.y - uy * head_len;
            let nx = -uy; let ny = ux;
            let p1 = (to.x, to.y);
            let p2 = (bx + nx * head_w * 0.5, by + ny * head_w * 0.5);
            let p3 = (bx - nx * head_w * 0.5, by - ny * head_w * 0.5);
            let mut hb = PathBuilder::new();
            hb.move_to(p1.0, p1.1);
            hb.line_to(p2.0, p2.1);
            hb.line_to(p3.0, p3.1);
            hb.close();
            if let Some(head) = hb.finish() {
                target.fill_path(&head, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
            }
        }
        Annotation::Ellipse { rect, stroke } => {
            if rect.is_degenerate() { return; }
            let r = match tiny_skia::Rect::from_xywh(rect.origin.x, rect.origin.y, rect.size.w, rect.size.h) {
                Some(r) => r,
                None => return,
            };
            let Some(path) = PathBuilder::from_oval(r) else { return };
            let paint = make_paint(stroke.color);
            let mut s = TsStroke::default();
            s.width = stroke.width.max(0.5);
            target.stroke_path(&path, &paint, &s, Transform::identity(), None);
        }
        Annotation::Pixelate { rect, tile_size } => {
            if rect.is_degenerate() || *tile_size == 0 { return; }
            let tw = target.width() as i32;
            let th = target.height() as i32;
            let x0 = (rect.origin.x.round() as i32).max(0);
            let y0 = (rect.origin.y.round() as i32).max(0);
            let x1 = ((rect.origin.x + rect.size.w).round() as i32).min(tw);
            let y1 = ((rect.origin.y + rect.size.h).round() as i32).min(th);
            if x1 <= x0 || y1 <= y0 { return; }
            let ts = *tile_size as i32;
            let src_data = source.data();
            let src_stride = source.width() as i32 * 4;

            let mut ty = y0;
            while ty < y1 {
                let mut tx = x0;
                while tx < x1 {
                    let bx1 = (tx + ts).min(x1);
                    let by1 = (ty + ts).min(y1);
                    let mut r_acc: u64 = 0;
                    let mut g_acc: u64 = 0;
                    let mut b_acc: u64 = 0;
                    let mut a_acc: u64 = 0;
                    let mut count: u64 = 0;
                    for py in ty..by1 {
                        for px in tx..bx1 {
                            let i = (py * src_stride + px * 4) as usize;
                            r_acc += src_data[i] as u64;
                            g_acc += src_data[i + 1] as u64;
                            b_acc += src_data[i + 2] as u64;
                            a_acc += src_data[i + 3] as u64;
                            count += 1;
                        }
                    }
                    if count == 0 { tx += ts; continue; }
                    let r = (r_acc / count) as u8;
                    let g = (g_acc / count) as u8;
                    let b = (b_acc / count) as u8;
                    let a = (a_acc / count) as u8;
                    let mut paint = tiny_skia::Paint::default();
                    paint.set_color_rgba8(r, g, b, a);
                    let r_ts = match tiny_skia::Rect::from_xywh(tx as f32, ty as f32, (bx1 - tx) as f32, (by1 - ty) as f32) {
                        Some(r) => r, None => { tx += ts; continue; }
                    };
                    target.fill_rect(r_ts, &paint, Transform::identity(), None);
                    tx += ts;
                }
                ty += ts;
            }
        }
        Annotation::Text { position, content, font_size, color } => {
            if content.is_empty() || *font_size <= 0.0 { return; }
            FONT_SYSTEM.with(|fs_cell| {
                SWASH_CACHE.with(|sc_cell| {
                    let mut fs = fs_cell.borrow_mut();
                    let mut sc = sc_cell.borrow_mut();
                    let metrics = Metrics::new(*font_size, *font_size * 1.2);
                    let mut buffer = Buffer::new(&mut fs, metrics);
                    buffer.set_size(
                        &mut fs,
                        Some(target.width() as f32 - position.x),
                        Some(target.height() as f32 - position.y),
                    );
                    buffer.set_text(&mut fs, content, &Attrs::new(), Shaping::Advanced, None);
                    buffer.shape_until_scroll(&mut fs, false);

                    let [r, g, b, a] = color.into_rgba8();
                    let cosmic_color = cosmic_text::Color::rgba(r, g, b, a);

                    let target_w = target.width() as i32;
                    let target_h = target.height() as i32;
                    let target_data = target.data_mut();
                    buffer.draw(&mut fs, &mut sc, cosmic_color, |gx, gy, _w, _h, gcolor| {
                        let px = position.x as i32 + gx;
                        let py = position.y as i32 + gy;
                        if px < 0 || py < 0 || px >= target_w || py >= target_h { return; }
                        let i = ((py * target_w + px) * 4) as usize;
                        let (sr, sg, sb, sa) = (gcolor.r(), gcolor.g(), gcolor.b(), gcolor.a());
                        if sa == 0 { return; }
                        // Premultiplied source over premultiplied dest.
                        let inv_a = 255 - sa as u32;
                        let pre = |c: u8| ((c as u32 * sa as u32 + 127) / 255) as u8;
                        let sr_p = pre(sr); let sg_p = pre(sg); let sb_p = pre(sb);
                        let blend = |s: u8, d: u8| ((s as u32 + (d as u32 * inv_a + 127) / 255).min(255)) as u8;
                        target_data[i]     = blend(sr_p, target_data[i]);
                        target_data[i + 1] = blend(sg_p, target_data[i + 1]);
                        target_data[i + 2] = blend(sb_p, target_data[i + 2]);
                        target_data[i + 3] = blend(sa, target_data[i + 3]);
                    });
                });
            });
        }
    }
}

fn make_paint(color: cosmic::iced::Color) -> tiny_skia::Paint<'static> {
    let mut p = tiny_skia::Paint::default();
    let [r, g, b, a] = color.into_rgba8();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

/// Apply scene.crop() to an RgbaImage. No-op if crop is None or degenerate.
// Casts: f32 origin/size values are clamped to non-negative finite ranges
// (and bounded by image extents below) before the as-cast to u32.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn apply_crop(img: RgbaImage, crop: Option<&LocalRect>) -> RgbaImage {
    let Some(r) = crop else { return img };
    let x = r.origin.x.round().max(0.0) as u32;
    let y = r.origin.y.round().max(0.0) as u32;
    let w = (r.size.w.round().max(0.0) as u32).min(img.width().saturating_sub(x));
    let h = (r.size.h.round().max(0.0) as u32).min(img.height().saturating_sub(y));
    if w == 0 || h == 0 { return img; }
    image::imageops::crop_imm(&img, x, y, w, h).to_image()
}

/// Compose scene over captured frame. Render fallback: if rendering panics or fails,
/// callers should fall back to the un-annotated capture. This function itself does not
/// catch panics; the caller wraps it with `std::panic::catch_unwind` (see screenshot.rs).
pub fn composite_annotations(captured: RgbaImage, scene: &AnnotationScene) -> RgbaImage {
    if scene.is_empty() {
        return captured;
    }
    let mut pix = pixmap_from_rgba(&captured);
    let src = pix.clone();
    render_annotations(&mut pix, &src, scene);
    let out = rgba_from_pixmap(&pix);
    apply_crop(out, scene.crop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use std::path::PathBuf;
    use crate::annotation::model::{Annotation, AnnotationScene, Color, LocalRect, Point, Size, Stroke};

    fn snapshot_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("src/annotation/tests/snapshots");
        p.push(format!("{name}.png"));
        p
    }

    fn assert_snapshot(name: &str, pix: &Pixmap) {
        let path = snapshot_path(name);
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() || !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            pix.save_png(&path).unwrap();
            return;
        }
        let actual = pix.encode_png().unwrap();
        let expected = std::fs::read(&path).unwrap();
        assert_eq!(actual, expected, "snapshot mismatch for {name} — re-run with UPDATE_SNAPSHOTS=1 to update");
    }

    fn solid_canvas(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut pix = Pixmap::new(w, h).unwrap();
        for chunk in pix.data_mut().chunks_exact_mut(4) {
            // Premultiply.
            let af = rgba[3] as u32;
            chunk[0] = ((rgba[0] as u32 * af + 127) / 255) as u8;
            chunk[1] = ((rgba[1] as u32 * af + 127) / 255) as u8;
            chunk[2] = ((rgba[2] as u32 * af + 127) / 255) as u8;
            chunk[3] = rgba[3];
        }
        pix
    }

    fn red_stroke(w: f32) -> Stroke {
        Stroke { width: w, color: Color::from_rgb(1.0, 0.0, 0.0) }
    }

    #[test]
    fn snapshot_pen_diagonal() {
        let mut canvas = solid_canvas(64, 64, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Pen {
            points: vec![
                Point { x: 8.0, y: 8.0 },
                Point { x: 32.0, y: 32.0 },
                Point { x: 56.0, y: 8.0 },
            ],
            stroke: red_stroke(3.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("pen_diagonal", &canvas);
    }

    #[test]
    fn snapshot_rect_outline() {
        let mut canvas = solid_canvas(64, 64, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Rectangle {
            rect: LocalRect { origin: Point { x: 12.0, y: 12.0 }, size: Size { w: 40.0, h: 40.0 } },
            stroke: red_stroke(2.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("rect_outline", &canvas);
    }

    #[test]
    fn snapshot_line_horizontal() {
        let mut canvas = solid_canvas(64, 64, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Line {
            from: Point { x: 8.0, y: 32.0 },
            to: Point { x: 56.0, y: 32.0 },
            stroke: red_stroke(2.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("line_horizontal", &canvas);
    }

    #[test]
    fn snapshot_arrow_diagonal() {
        let mut canvas = solid_canvas(64, 64, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Arrow {
            from: Point { x: 8.0, y: 8.0 },
            to: Point { x: 56.0, y: 56.0 },
            stroke: red_stroke(3.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("arrow_diagonal", &canvas);
    }

    #[test]
    fn snapshot_ellipse_outline() {
        let mut canvas = solid_canvas(64, 64, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Ellipse {
            rect: LocalRect { origin: Point { x: 8.0, y: 16.0 }, size: Size { w: 48.0, h: 32.0 } },
            stroke: red_stroke(2.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("ellipse_outline", &canvas);
    }

    #[test]
    fn round_trip_opaque_solid() {
        let mut img = RgbaImage::new(4, 4);
        for p in img.pixels_mut() { *p = Rgba([200, 100, 50, 255]); }
        let pix = pixmap_from_rgba(&img);
        let back = rgba_from_pixmap(&pix);
        assert_eq!(back.get_pixel(0, 0), &Rgba([200, 100, 50, 255]));
    }

    #[test]
    fn round_trip_translucent() {
        let mut img = RgbaImage::new(2, 2);
        for p in img.pixels_mut() { *p = Rgba([200, 100, 50, 128]); }
        let pix = pixmap_from_rgba(&img);
        let back = rgba_from_pixmap(&pix);
        // Allow ±1 due to integer rounding in 8-bit premultiplication round-trip.
        let p = back.get_pixel(0, 0);
        assert!((p[0] as i32 - 200).abs() <= 1);
        assert!((p[1] as i32 - 100).abs() <= 1);
        assert!((p[2] as i32 - 50).abs() <= 1);
        assert_eq!(p[3], 128);
    }

    #[test]
    fn composite_empty_scene_returns_input() {
        let mut img = RgbaImage::new(4, 4);
        for p in img.pixels_mut() { *p = Rgba([10, 20, 30, 255]); }
        let scene = AnnotationScene::default();
        let out = composite_annotations(img.clone(), &scene);
        assert_eq!(img, out);
    }

    #[test]
    fn apply_crop_extracts_subregion() {
        let mut img = RgbaImage::new(10, 10);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgba([x as u8, y as u8, 0, 255]);
        }
        let crop = LocalRect { origin: super::super::model::Point { x: 2.0, y: 3.0 }, size: super::super::model::Size { w: 4.0, h: 5.0 } };
        let out = apply_crop(img, Some(&crop));
        assert_eq!(out.dimensions(), (4, 5));
        assert_eq!(out.get_pixel(0, 0), &Rgba([2, 3, 0, 255]));
    }

    #[test]
    fn apply_crop_none_passthrough() {
        let img = RgbaImage::new(10, 10);
        let out = apply_crop(img.clone(), None);
        assert_eq!(out.dimensions(), (10, 10));
    }

    #[test]
    fn snapshot_pixelate_grid() {
        // Source: 32x32 image with a checkerboard pattern at 1px granularity.
        let mut canvas = Pixmap::new(32, 32).unwrap();
        for y in 0..32u32 {
            for x in 0..32u32 {
                let on = (x + y) % 2 == 0;
                let i = ((y * 32 + x) * 4) as usize;
                let v = if on { 255 } else { 0 };
                canvas.data_mut()[i] = v;
                canvas.data_mut()[i + 1] = v;
                canvas.data_mut()[i + 2] = v;
                canvas.data_mut()[i + 3] = 255;
            }
        }
        let src = canvas.clone();

        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Pixelate {
            rect: LocalRect { origin: Point { x: 0.0, y: 0.0 }, size: Size { w: 32.0, h: 32.0 } },
            tile_size: 8,
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        // After 8x8 tile averaging of a 1px checkerboard, every tile averages to ~128.
        assert_snapshot("pixelate_grid", &canvas);
    }

    #[test]
    fn snapshot_text_hello() {
        let mut canvas = solid_canvas(128, 32, [255, 255, 255, 255]);
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Text {
            position: Point { x: 8.0, y: 8.0 },
            content: "Hello".to_string(),
            font_size: 16.0,
            color: Color::from_rgb(0.0, 0.0, 0.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_snapshot("text_hello", &canvas);
    }

    #[test]
    fn empty_text_is_noop() {
        let mut canvas = solid_canvas(8, 8, [255, 255, 255, 255]);
        let before = canvas.data().to_vec();
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Text {
            position: Point::default(),
            content: String::new(),
            font_size: 16.0,
            color: Color::from_rgb(0.0, 0.0, 0.0),
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_eq!(canvas.data(), before.as_slice());
    }

    #[test]
    fn pixelate_zero_tile_size_is_noop() {
        let mut canvas = solid_canvas(8, 8, [100, 100, 100, 255]);
        let before = canvas.data().to_vec();
        let src = canvas.clone();
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Pixelate {
            rect: LocalRect { origin: Point { x: 0.0, y: 0.0 }, size: Size { w: 8.0, h: 8.0 } },
            tile_size: 0,
        });
        scene.commit_in_progress();
        render_annotations(&mut canvas, &src, &scene);
        assert_eq!(canvas.data(), before.as_slice());
    }

    #[test]
    fn composite_without_crop_preserves_dimensions() {
        let mut img = RgbaImage::new(100, 80);
        for p in img.pixels_mut() { *p = Rgba([0, 0, 0, 255]); }
        let mut scene = AnnotationScene::default();
        scene.begin(Annotation::Pen {
            points: vec![Point { x: 5.0, y: 5.0 }, Point { x: 50.0, y: 50.0 }],
            stroke: Stroke { width: 2.0, color: Color::from_rgb(1.0, 0.0, 0.0) },
        });
        scene.commit_in_progress();
        let out = composite_annotations(img, &scene);
        assert_eq!(out.dimensions(), (100, 80));
    }

    #[test]
    fn composite_with_crop_dimensions_match_crop() {
        let mut img = RgbaImage::new(100, 80);
        for p in img.pixels_mut() { *p = Rgba([0, 0, 0, 255]); }
        let mut scene = AnnotationScene::default();
        scene.set_crop(Some(LocalRect { origin: Point { x: 10.0, y: 5.0 }, size: Size { w: 40.0, h: 30.0 } }));
        let out = composite_annotations(img, &scene);
        assert_eq!(out.dimensions(), (40, 30));
    }
}
