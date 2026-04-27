#![allow(dead_code, unused_variables)]

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::widget::Stack;
use cosmic::iced_core::Point as IcedPoint;
use cosmic::widget::{button, column, container, mouse_area, row, space};
use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::annotation::model::{Annotation, AnnotationScene, Point, Stroke, Tool, ToolState};
use crate::annotation::render::{pixmap_from_rgba, render_annotations, rgba_from_pixmap};
use crate::fl;

pub struct AnnotationView {
    pub captured: RgbaImage,
    pub captured_handle: cosmic::widget::image::Handle,
    pub scene: AnnotationScene,
    pub tools: ToolState,
    pub overlay_handle: Option<cosmic::widget::image::Handle>,
    source_pixmap: Option<Pixmap>,
    pointer_down: Option<Point>,
    /// Last cursor position seen via on_move, in widget-local logical pixels.
    last_cursor: Option<IcedPoint>,
    /// Logical-pixel size of the rendered canvas widget. When (0,0), coordinates are mapped
    /// 1:1 from widget pixels to canvas pixels.
    canvas_widget_size: (f32, f32),
}

impl AnnotationView {
    pub fn new(captured: RgbaImage) -> Self {
        let captured_handle = cosmic::widget::image::Handle::from_rgba(
            captured.width(),
            captured.height(),
            captured.clone().into_vec(),
        );
        Self {
            captured,
            captured_handle,
            scene: AnnotationScene::default(),
            tools: ToolState::default(),
            overlay_handle: None,
            source_pixmap: None,
            pointer_down: None,
            last_cursor: None,
            canvas_widget_size: (0.0, 0.0),
        }
    }

    fn map_pointer(&self, p: IcedPoint) -> Point {
        let (vw, vh) = self.canvas_widget_size;
        if vw <= 0.0 || vh <= 0.0 {
            return Point { x: p.x, y: p.y };
        }
        let scale_x = self.captured.width() as f32 / vw;
        let scale_y = self.captured.height() as f32 / vh;
        Point {
            x: p.x * scale_x,
            y: p.y * scale_y,
        }
    }

    fn ensure_source(&mut self) -> &Pixmap {
        if self.source_pixmap.is_none() {
            self.source_pixmap = Some(pixmap_from_rgba(&self.captured));
        }
        self.source_pixmap.as_ref().unwrap()
    }

    pub fn invalidate_overlay(&mut self) {
        if self.scene.is_empty() {
            self.overlay_handle = None;
            return;
        }
        let w = self.captured.width();
        let h = self.captured.height();
        let mut target = Pixmap::new(w, h).expect("non-zero pixmap");
        let source = self.ensure_source().clone();
        render_annotations(&mut target, &source, &self.scene);
        let rgba = rgba_from_pixmap(&target);
        self.overlay_handle = Some(cosmic::widget::image::Handle::from_rgba(
            w,
            h,
            rgba.into_vec(),
        ));
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Done,
    Cancel,
    SelectTool(Tool),
    Undo,
    Redo,
    PointerDown,
    PointerMove(IcedPoint),
    PointerUp,
    CanvasResized { width: f32, height: f32 },
}

pub fn view(state: &AnnotationView) -> Element<'_, Msg> {
    let toolbar = build_toolbar(state);

    let bg: Element<'_, Msg> = cosmic::widget::image(state.captured_handle.clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    let overlay: Element<'_, Msg> = match &state.overlay_handle {
        Some(h) => cosmic::widget::image(h.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        None => space::horizontal().width(Length::Fill).into(),
    };

    let canvas = Stack::with_children(vec![bg, overlay])
        .width(Length::Fill)
        .height(Length::Fill);
    let canvas = mouse_area(canvas)
        .on_press(Msg::PointerDown)
        .on_move(Msg::PointerMove)
        .on_release(Msg::PointerUp);

    column::with_children(vec![
        toolbar,
        container(canvas)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    ])
    .into()
}

fn build_toolbar(state: &AnnotationView) -> Element<'_, Msg> {
    let tool_btn = |label: String, t: Tool| {
        let mut b = button::standard(label).on_press(Msg::SelectTool(t));
        if state.tools.active_tool == t {
            b = b.class(cosmic::theme::Button::Suggested);
        }
        b
    };

    row::with_children(vec![
        tool_btn(fl!("tool-pen"), Tool::Pen).into(),
        space::horizontal().width(Length::Fill).into(),
        button::standard(fl!("annotate-undo"))
            .on_press(Msg::Undo)
            .into(),
        button::standard(fl!("annotate-redo"))
            .on_press(Msg::Redo)
            .into(),
        button::standard(fl!("annotate-cancel"))
            .on_press(Msg::Cancel)
            .into(),
        button::suggested(fl!("annotate-done"))
            .on_press(Msg::Done)
            .into(),
    ])
    .spacing(8)
    .padding(8)
    .into()
}

pub enum UpdateOutcome {
    None,
    Done,
    Cancel,
}

pub fn update(state: &mut AnnotationView, msg: Msg) -> UpdateOutcome {
    match msg {
        Msg::Done => UpdateOutcome::Done,
        Msg::Cancel => UpdateOutcome::Cancel,
        Msg::SelectTool(t) => {
            state.tools.active_tool = t;
            UpdateOutcome::None
        }
        Msg::Undo => {
            state.scene.undo();
            state.invalidate_overlay();
            UpdateOutcome::None
        }
        Msg::Redo => {
            state.scene.redo();
            state.invalidate_overlay();
            UpdateOutcome::None
        }
        Msg::CanvasResized { width, height } => {
            state.canvas_widget_size = (width, height);
            UpdateOutcome::None
        }
        Msg::PointerDown => {
            let Some(p) = state.last_cursor else {
                return UpdateOutcome::None;
            };
            let cp = state.map_pointer(p);
            state.pointer_down = Some(cp);
            match state.tools.active_tool {
                Tool::Pen => {
                    state.scene.begin(Annotation::Pen {
                        points: vec![cp],
                        stroke: Stroke {
                            width: state.tools.stroke_width,
                            color: state.tools.color,
                        },
                    });
                }
                _ => {}
            }
            UpdateOutcome::None
        }
        Msg::PointerMove(p) => {
            state.last_cursor = Some(p);
            if state.pointer_down.is_none() {
                return UpdateOutcome::None;
            }
            let cp = state.map_pointer(p);
            match state.tools.active_tool {
                Tool::Pen => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Pen { points, .. } = a {
                            points.push(cp);
                        }
                    });
                    state.invalidate_overlay();
                }
                _ => {}
            }
            UpdateOutcome::None
        }
        Msg::PointerUp => {
            if state.pointer_down.take().is_none() {
                return UpdateOutcome::None;
            }
            state.scene.commit_in_progress();
            state.invalidate_overlay();
            UpdateOutcome::None
        }
    }
}
