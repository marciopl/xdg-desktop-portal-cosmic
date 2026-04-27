#![allow(dead_code, unused_variables)]

use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::annotation::model::{AnnotationScene, LocalRect};

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

pub fn render_annotations(_target: &mut Pixmap, _source: &Pixmap, _scene: &AnnotationScene) {
    // Implemented in Tasks 6–9.
}

/// Apply scene.crop() to an RgbaImage. No-op if crop is None or degenerate.
pub fn apply_crop(img: RgbaImage, crop: Option<&LocalRect>) -> RgbaImage {
    let Some(r) = crop else { return img };
    let x = r.origin.x.round().max(0.0) as u32;
    let y = r.origin.y.round().max(0.0) as u32;
    let w = (r.size.w.round() as u32).min(img.width().saturating_sub(x));
    let h = (r.size.h.round() as u32).min(img.height().saturating_sub(y));
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
}
