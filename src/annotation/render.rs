#![allow(dead_code, unused_variables)]
// Implemented in Tasks 4–9.

use image::RgbaImage;
use crate::annotation::model::AnnotationScene;

pub fn render_annotations(_target: &mut tiny_skia::Pixmap, _source: &tiny_skia::Pixmap, _scene: &AnnotationScene) {}

pub fn composite_annotations(captured: RgbaImage, _scene: &AnnotationScene) -> RgbaImage {
    captured
}
