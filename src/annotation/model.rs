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

#[derive(Clone, Debug)]
enum Op {
    AddItem,
    SetCrop { previous: Option<LocalRect> },
}

#[derive(Clone, Debug)]
enum RedoData {
    Item(Annotation),
    Crop(Option<LocalRect>),
}

#[derive(Clone, Debug, Default)]
pub struct AnnotationScene {
    items: Vec<Annotation>,
    in_progress: Option<Annotation>,
    crop: Option<LocalRect>,
    history: Vec<Op>,
    redo_stack: Vec<(Op, RedoData)>,
}

impl AnnotationScene {
    pub fn begin(&mut self, ann: Annotation) {
        self.in_progress = Some(ann);
    }

    pub fn update_in_progress<F: FnOnce(&mut Annotation)>(&mut self, f: F) {
        if let Some(ann) = self.in_progress.as_mut() {
            f(ann);
        }
    }

    pub fn commit_in_progress(&mut self) {
        if let Some(ann) = self.in_progress.take() {
            self.items.push(ann);
            self.history.push(Op::AddItem);
            self.redo_stack.clear();
        }
    }

    pub fn cancel_in_progress(&mut self) {
        self.in_progress = None;
    }

    pub fn set_crop(&mut self, rect: Option<LocalRect>) {
        let previous = self.crop;
        self.crop = rect;
        self.history.push(Op::SetCrop { previous });
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        let Some(op) = self.history.pop() else { return };
        match op {
            Op::AddItem => {
                if let Some(removed) = self.items.pop() {
                    self.redo_stack.push((Op::AddItem, RedoData::Item(removed)));
                }
            }
            Op::SetCrop { previous } => {
                let current = self.crop;
                self.crop = previous;
                self.redo_stack.push((Op::SetCrop { previous }, RedoData::Crop(current)));
            }
        }
    }

    pub fn redo(&mut self) {
        let Some((op, data)) = self.redo_stack.pop() else { return };
        match (op, data) {
            (Op::AddItem, RedoData::Item(ann)) => {
                self.items.push(ann);
                self.history.push(Op::AddItem);
            }
            (Op::SetCrop { .. }, RedoData::Crop(target)) => {
                let previous = self.crop;
                self.crop = target;
                self.history.push(Op::SetCrop { previous });
            }
            _ => unreachable!("redo data shape mismatched op"),
        }
    }

    pub fn iter_committed(&self) -> impl Iterator<Item = &Annotation> {
        self.items.iter()
    }

    pub fn in_progress(&self) -> Option<&Annotation> {
        self.in_progress.as_ref()
    }

    pub fn crop(&self) -> Option<&LocalRect> {
        self.crop.as_ref()
    }

    /// True if there's nothing to render and no crop — used to skip allocating an overlay.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.in_progress.is_none() && self.crop.is_none()
    }
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

    fn pen() -> Annotation {
        Annotation::Pen { points: vec![Point::default()], stroke: Stroke { width: 1.0, color: black() } }
    }

    #[test]
    fn begin_sets_in_progress() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        assert!(s.in_progress().is_some());
        assert_eq!(s.iter_committed().count(), 0);
    }

    #[test]
    fn commit_in_progress_moves_to_items() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        s.commit_in_progress();
        assert!(s.in_progress().is_none());
        assert_eq!(s.iter_committed().count(), 1);
    }

    #[test]
    fn cancel_in_progress_drops_it() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        s.cancel_in_progress();
        assert!(s.in_progress().is_none());
        assert_eq!(s.iter_committed().count(), 0);
    }

    #[test]
    fn undo_after_commit_removes_item() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        s.commit_in_progress();
        s.undo();
        assert_eq!(s.iter_committed().count(), 0);
    }

    #[test]
    fn redo_after_undo_restores_item() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        s.commit_in_progress();
        s.undo();
        s.redo();
        assert_eq!(s.iter_committed().count(), 1);
    }

    #[test]
    fn new_commit_clears_redo_stack() {
        let mut s = AnnotationScene::default();
        s.begin(pen()); s.commit_in_progress();
        s.undo();
        s.begin(pen()); s.commit_in_progress();
        s.redo(); // should be a no-op
        assert_eq!(s.iter_committed().count(), 1);
    }

    #[test]
    fn update_in_progress_mutates_only_in_progress() {
        let mut s = AnnotationScene::default();
        s.begin(pen());
        s.update_in_progress(|a| {
            if let Annotation::Pen { points, .. } = a {
                points.push(Point { x: 1.0, y: 1.0 });
            }
        });
        if let Some(Annotation::Pen { points, .. }) = s.in_progress() {
            assert_eq!(points.len(), 2);
        } else { panic!("in_progress lost") }
    }

    #[test]
    fn undo_with_no_history_is_noop() {
        let mut s = AnnotationScene::default();
        s.undo(); s.redo();
        assert_eq!(s.iter_committed().count(), 0);
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> LocalRect {
        LocalRect { origin: Point { x, y }, size: Size { w, h } }
    }

    #[test]
    fn set_crop_records_history() {
        let mut s = AnnotationScene::default();
        s.set_crop(Some(rect(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(s.crop(), Some(&rect(0.0, 0.0, 100.0, 100.0)));
        s.undo();
        assert_eq!(s.crop(), None);
    }

    #[test]
    fn redo_restores_crop() {
        let mut s = AnnotationScene::default();
        s.set_crop(Some(rect(0.0, 0.0, 100.0, 100.0)));
        s.undo();
        s.redo();
        assert_eq!(s.crop(), Some(&rect(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn crop_then_resize_undo_returns_to_first_crop() {
        let mut s = AnnotationScene::default();
        s.set_crop(Some(rect(0.0, 0.0, 100.0, 100.0)));
        s.set_crop(Some(rect(0.0, 0.0, 50.0, 50.0)));
        s.undo();
        assert_eq!(s.crop(), Some(&rect(0.0, 0.0, 100.0, 100.0)));
        s.undo();
        assert_eq!(s.crop(), None);
    }

    #[test]
    fn interleaved_item_and_crop_undo_redo() {
        let mut s = AnnotationScene::default();
        s.begin(pen()); s.commit_in_progress();
        s.set_crop(Some(rect(0.0, 0.0, 50.0, 50.0)));
        s.begin(pen()); s.commit_in_progress();

        // 2 items + crop set
        assert_eq!(s.iter_committed().count(), 2);
        assert!(s.crop().is_some());

        s.undo(); // remove second item
        assert_eq!(s.iter_committed().count(), 1);
        assert!(s.crop().is_some());

        s.undo(); // remove crop
        assert_eq!(s.iter_committed().count(), 1);
        assert_eq!(s.crop(), None);

        s.undo(); // remove first item
        assert_eq!(s.iter_committed().count(), 0);

        s.redo(); s.redo(); s.redo();
        assert_eq!(s.iter_committed().count(), 2);
        assert!(s.crop().is_some());
    }

    #[test]
    fn set_crop_none_clears_crop() {
        let mut s = AnnotationScene::default();
        s.set_crop(Some(rect(0.0, 0.0, 100.0, 100.0)));
        s.set_crop(None);
        assert_eq!(s.crop(), None);
        s.undo();
        assert_eq!(s.crop(), Some(&rect(0.0, 0.0, 100.0, 100.0)));
    }
}
