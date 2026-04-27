#![allow(dead_code)]

pub use cosmic::iced::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point { pub x: f32, pub y: f32 }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size { pub w: f32, pub h: f32 }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalRect { pub origin: Point, pub size: Size }

impl LocalRect {
    /// Build a rect from any two corner points; result always has non-negative w/h.
    pub fn from_corners(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        let w = (a.x - b.x).abs();
        let h = (a.y - b.y).abs();
        Self { origin: Point { x, y }, size: Size { w, h } }
    }

    /// Hit test. Inclusive on all four edges — a point exactly on the right/bottom edge counts as inside.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x
            && p.y >= self.origin.y
            && p.x <= self.origin.x + self.size.w
            && p.y <= self.origin.y + self.size.h
    }

    /// True when either extent is below half a pixel — too small to render meaningfully.
    /// Used as a render-time guard against zero-size shapes from click-without-drag input.
    pub fn is_degenerate(&self) -> bool {
        self.size.w <= 0.5 || self.size.h <= 0.5
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke { pub width: f32, pub color: Color }

#[derive(Clone, Debug)]
pub enum Annotation {
    Pen       { points: Vec<Point>, stroke: Stroke },
    Line      { from: Point, to: Point, stroke: Stroke },
    Arrow     { from: Point, to: Point, stroke: Stroke },
    Rectangle { rect: LocalRect, stroke: Stroke },
    Ellipse   { rect: LocalRect, stroke: Stroke },
    Text      { position: Point, content: String, font_size: f32, color: Color },
    Pixelate  { rect: LocalRect, tile_size: u32 },
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pen,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Text,
    Pixelate,
    Crop,
}

#[derive(Clone, Debug)]
pub struct ToolState {
    pub active_tool: Tool,
    pub color: Color,
    pub stroke_width: f32,   // clamped to 1..=32
    pub text_size: f32,      // points; default 16
    pub tile_size: u32,      // pixelate tile px; default 16, min 4
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            active_tool: Tool::default(),
            color: Color::from_rgb(1.0, 0.0, 0.0), // red — the canonical annotator default
            stroke_width: 4.0,
            text_size: 16.0,
            tile_size: 16,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnnotationScene {
    // Implemented in Task 3.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black() -> Color { Color::from_rgb(0.0, 0.0, 0.0) }

    #[test]
    fn point_default_is_origin() {
        assert_eq!(Point::default(), Point { x: 0.0, y: 0.0 });
    }

    #[test]
    fn local_rect_normalizes_negative_size() {
        let r = LocalRect::from_corners(Point { x: 10.0, y: 20.0 }, Point { x: 5.0, y: 5.0 });
        assert_eq!(r.origin, Point { x: 5.0, y: 5.0 });
        assert_eq!(r.size, Size { w: 5.0, h: 15.0 });
    }

    #[test]
    fn local_rect_contains() {
        let r = LocalRect { origin: Point { x: 0.0, y: 0.0 }, size: Size { w: 10.0, h: 10.0 } };
        assert!(r.contains(Point { x: 5.0, y: 5.0 }));
        assert!(!r.contains(Point { x: 15.0, y: 5.0 }));
    }

    #[test]
    fn annotation_pen_carries_stroke() {
        let stroke = Stroke { width: 4.0, color: black() };
        let ann = Annotation::Pen { points: vec![Point::default()], stroke };
        match ann {
            Annotation::Pen { stroke: s, .. } => assert_eq!(s.width, 4.0),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn tool_state_default_values_in_range() {
        let s = ToolState::default();
        assert!(s.stroke_width >= 1.0 && s.stroke_width <= 32.0);
        assert!(s.text_size > 0.0);
        assert!(s.tile_size >= 4);
    }
}
