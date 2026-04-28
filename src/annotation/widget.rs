#![allow(dead_code, unused_variables)]

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::widget::Stack;
use cosmic::iced_core::Point as IcedPoint;
use cosmic::widget::{button, column, container, icon, mouse_area, row, space, text};
use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::annotation::model::{
    Annotation, AnnotationScene, Color, LocalRect, Point, Size, Stroke, Tool, ToolState,
};
use crate::annotation::render::{pixmap_from_rgba, render_annotations, rgba_from_pixmap};
use crate::fl;

pub struct TextEditState {
    pub position: Point,
    pub text: String,
    pub input_id: cosmic::widget::Id,
}

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
    /// Pending text edit: position (canvas-local), current text buffer, focus id.
    pub text_edit: Option<TextEditState>,
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
            text_edit: None,
        }
    }

    fn map_pointer(&self, p: IcedPoint) -> Point {
        let (vw, vh) = self.canvas_widget_size;
        // TODO(task-11): canvas_widget_size is set to (0,0) until the parent plumbs
        // a window-resize signal into Msg::CanvasResized. The 1:1 fallback is correct
        // only when the layer surface renders at native (physical) resolution. On HiDPI
        // outputs with non-1.0 scale this will misalign strokes — verify on Task 11.
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

    pub fn invalidate_overlay(&mut self) {
        if self.scene.is_empty() {
            self.overlay_handle = None;
            return;
        }
        let w = self.captured.width();
        let h = self.captured.height();
        // Lazily fill source_pixmap WITHOUT holding a borrow into self afterwards.
        if self.source_pixmap.is_none() {
            self.source_pixmap = Some(pixmap_from_rgba(&self.captured));
        }
        let mut target = Pixmap::new(w, h).expect("non-zero pixmap");
        let source = self.source_pixmap.as_ref().unwrap();
        render_annotations(&mut target, source, &self.scene);
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
    ResetCrop,
    PointerDown,
    PointerMove(IcedPoint),
    PointerUp,
    CanvasResized { width: f32, height: f32 },
    TextEditChanged(String),
    TextEditSubmit,
    TextEditCancel,
    SetColor(Color),
    SetStrokeWidth(f32),
    SetTextSize(f32),
    SetTileSize(u32),
}

pub fn view(state: &AnnotationView) -> Element<'_, Msg> {
    let toolbar = build_toolbar(state);

    // Render the canvas at the captured image's exact pixel size so widget
    // coordinates equal canvas-local pixels (1:1 mapping, no scaling).
    let img_w = Length::Fixed(state.captured.width() as f32);
    let img_h = Length::Fixed(state.captured.height() as f32);

    let bg: Element<'_, Msg> = cosmic::widget::image(state.captured_handle.clone())
        .width(img_w)
        .height(img_h)
        .into();
    let overlay: Element<'_, Msg> = match &state.overlay_handle {
        Some(h) => cosmic::widget::image(h.clone())
            .width(img_w)
            .height(img_h)
            .into(),
        None => space::horizontal().width(Length::Fixed(0.0)).into(),
    };

    let canvas_stack = Stack::with_children(vec![bg, overlay])
        .width(img_w)
        .height(img_h);
    let canvas_area = mouse_area(canvas_stack)
        .on_press(Msg::PointerDown)
        .on_move(Msg::PointerMove)
        .on_release(Msg::PointerUp);

    // If a text edit is in progress, layer a positioned text_input over the canvas.
    let canvas_element: Element<'_, Msg> = if let Some(te) = &state.text_edit {
        let leading_x = te.position.x.max(0.0);
        let leading_y = te.position.y.max(0.0);
        let input: Element<'_, Msg> = cosmic::widget::text_input("", &te.text)
            .id(te.input_id.clone())
            .on_input(Msg::TextEditChanged)
            .on_submit(|_| Msg::TextEditSubmit)
            .width(Length::Fixed(200.0))
            .into();
        let positioned: Element<'_, Msg> = column::with_children(vec![
            space::vertical().height(Length::Fixed(leading_y)).into(),
            row::with_children(vec![
                space::horizontal().width(Length::Fixed(leading_x)).into(),
                input,
            ])
            .into(),
        ])
        .into();
        Stack::with_children(vec![canvas_area.into(), positioned])
            .width(img_w)
            .height(img_h)
            .into()
    } else {
        canvas_area.into()
    };

    column::with_children(vec![toolbar, canvas_element])
        .into()
}

const TOOLBAR_ICON_SIZE: u16 = 18;

fn icon_tool_btn<'a>(
    state: &AnnotationView,
    icon_name: &'static str,
    label: String,
    shortcut: &'static str,
    tool: Tool,
) -> Element<'a, Msg> {
    let icon_widget = icon::Icon::from(icon::from_name(icon_name).size(TOOLBAR_ICON_SIZE));
    let mut btn = button::custom(icon_widget).on_press(Msg::SelectTool(tool));
    if state.tools.active_tool == tool {
        btn = btn.class(cosmic::theme::Button::Suggested);
    }
    cosmic::widget::tooltip::tooltip(
        btn,
        text(format!("{label} ({shortcut})")),
        cosmic::widget::tooltip::Position::Bottom,
    )
    .into()
}

fn icon_action_btn<'a>(
    icon_name: &'static str,
    label: String,
    shortcut: &'static str,
    on_press: Msg,
) -> Element<'a, Msg> {
    let icon_widget = icon::Icon::from(icon::from_name(icon_name).size(TOOLBAR_ICON_SIZE));
    let btn = button::custom(icon_widget).on_press(on_press);
    cosmic::widget::tooltip::tooltip(
        btn,
        text(format!("{label} ({shortcut})")),
        cosmic::widget::tooltip::Position::Bottom,
    )
    .into()
}

fn build_toolbar<'a>(state: &'a AnnotationView) -> Element<'a, Msg> {
    let tools = row::with_children(vec![
        icon_tool_btn(state, "pencil-symbolic", fl!("tool-pen"), "P", Tool::Pen),
        icon_tool_btn(
            state,
            "insert-line-symbolic",
            fl!("tool-line"),
            "L",
            Tool::Line,
        ),
        icon_tool_btn(
            state,
            "insert-arrow-symbolic",
            fl!("tool-arrow"),
            "A",
            Tool::Arrow,
        ),
        icon_tool_btn(
            state,
            "insert-rectangle-symbolic",
            fl!("tool-rectangle"),
            "R",
            Tool::Rectangle,
        ),
        icon_tool_btn(
            state,
            "insert-ellipse-symbolic",
            fl!("tool-ellipse"),
            "E",
            Tool::Ellipse,
        ),
        icon_tool_btn(
            state,
            "insert-text-symbolic",
            fl!("tool-text"),
            "T",
            Tool::Text,
        ),
        icon_tool_btn(
            state,
            "image-red-eye-symbolic",
            fl!("tool-pixelate"),
            "B",
            Tool::Pixelate,
        ),
        icon_tool_btn(
            state,
            "image-crop-rotate-symbolic",
            fl!("tool-crop"),
            "C",
            Tool::Crop,
        ),
    ])
    .spacing(4);

    let palette: Vec<Element<'_, Msg>> = PALETTE
        .iter()
        .map(|c| color_swatch(*c, color_eq(*c, state.tools.color)))
        .collect();
    let palette_row = row::with_children(palette).spacing(4);

    let stroke = stepper(
        fl!("stroke-width"),
        format!("{:.0}", state.tools.stroke_width),
        Msg::SetStrokeWidth(state.tools.stroke_width - 1.0),
        Msg::SetStrokeWidth(state.tools.stroke_width + 1.0),
    );

    let tool_specific: Element<'_, Msg> = match state.tools.active_tool {
        Tool::Text => stepper(
            fl!("text-size"),
            format!("{:.0}", state.tools.text_size),
            Msg::SetTextSize(state.tools.text_size - 2.0),
            Msg::SetTextSize(state.tools.text_size + 2.0),
        ),
        Tool::Pixelate => stepper(
            fl!("tile-size"),
            format!("{}", state.tools.tile_size),
            Msg::SetTileSize(state.tools.tile_size.saturating_sub(2)),
            Msg::SetTileSize(state.tools.tile_size + 2),
        ),
        Tool::Crop => button::standard(fl!("annotate-reset-crop"))
            .on_press(Msg::ResetCrop)
            .into(),
        _ => space::horizontal().width(Length::Fixed(0.0)).into(),
    };

    let history_and_exit = row::with_children(vec![
        icon_action_btn(
            "edit-undo-symbolic",
            fl!("annotate-undo"),
            "Ctrl+Z",
            Msg::Undo,
        ),
        icon_action_btn(
            "edit-redo-symbolic",
            fl!("annotate-redo"),
            "Ctrl+Shift+Z",
            Msg::Redo,
        ),
        icon_action_btn(
            "window-close-symbolic",
            fl!("annotate-cancel"),
            "Esc",
            Msg::Cancel,
        ),
        button::suggested(fl!("annotate-done"))
            .on_press(Msg::Done)
            .into(),
    ])
    .spacing(8);

    column::with_children(vec![
        tools.into(),
        row::with_children(vec![
            palette_row.into(),
            stroke,
            tool_specific,
            space::horizontal().width(Length::Fill).into(),
            history_and_exit.into(),
        ])
        .spacing(16)
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
        Msg::ResetCrop => {
            state.scene.set_crop(None);
            state.invalidate_overlay();
            UpdateOutcome::None
        }
        Msg::CanvasResized { width, height } => {
            state.canvas_widget_size = (width, height);
            UpdateOutcome::None
        }
        Msg::PointerDown => {
            // TODO(task-14): handle touch/stylus where PointerDown can fire before any
            // PointerMove. iced's mouse backend emits CursorMoved first for mice, so this
            // works for the MVP, but a fresh tap that lands directly on the canvas without
            // a hover would be silently dropped here.
            let Some(p) = state.last_cursor else {
                return UpdateOutcome::None;
            };
            let cp = state.map_pointer(p);
            state.pointer_down = Some(cp);
            let stroke = Stroke {
                width: state.tools.stroke_width,
                color: state.tools.color,
            };
            let zero_rect = LocalRect {
                origin: cp,
                size: Size { w: 0.0, h: 0.0 },
            };
            match state.tools.active_tool {
                Tool::Pen => {
                    state.scene.begin(Annotation::Pen {
                        points: vec![cp],
                        stroke,
                    });
                }
                Tool::Line => {
                    state.scene.begin(Annotation::Line {
                        from: cp,
                        to: cp,
                        stroke,
                    });
                }
                Tool::Arrow => {
                    state.scene.begin(Annotation::Arrow {
                        from: cp,
                        to: cp,
                        stroke,
                    });
                }
                Tool::Rectangle => {
                    state.scene.begin(Annotation::Rectangle {
                        rect: zero_rect,
                        stroke,
                    });
                }
                Tool::Ellipse => {
                    state.scene.begin(Annotation::Ellipse {
                        rect: zero_rect,
                        stroke,
                    });
                }
                Tool::Pixelate => {
                    state.scene.begin(Annotation::Pixelate {
                        rect: zero_rect,
                        tile_size: state.tools.tile_size,
                    });
                }
                Tool::Text => {
                    state.text_edit = Some(TextEditState {
                        position: cp,
                        text: String::new(),
                        input_id: cosmic::widget::Id::unique(),
                    });
                    state.pointer_down = None; // Text doesn't drag.
                }
                Tool::Crop => {
                    state.scene.begin(Annotation::Rectangle {
                        rect: LocalRect {
                            origin: cp,
                            size: Size { w: 0.0, h: 0.0 },
                        },
                        stroke: Stroke {
                            width: 2.0,
                            color: state.tools.color,
                        },
                    });
                }
            }
            UpdateOutcome::None
        }
        Msg::PointerMove(p) => {
            state.last_cursor = Some(p);
            let Some(start) = state.pointer_down else {
                return UpdateOutcome::None;
            };
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
                Tool::Line => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Line { to, .. } = a {
                            *to = cp;
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Arrow => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Arrow { to, .. } = a {
                            *to = cp;
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Rectangle => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Rectangle { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Ellipse => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Ellipse { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Pixelate => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Pixelate { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Crop => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Rectangle { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                    state.invalidate_overlay();
                }
                Tool::Text => {}
            }
            UpdateOutcome::None
        }
        Msg::PointerUp => {
            // Always clear the cached down position, regardless of whether we commit.
            if state.pointer_down.take().is_none() {
                return UpdateOutcome::None;
            }
            if state.tools.active_tool == Tool::Crop {
                let rect = match state.scene.in_progress() {
                    Some(Annotation::Rectangle { rect, .. }) if !rect.is_degenerate() => Some(*rect),
                    _ => None,
                };
                state.scene.cancel_in_progress();
                if let Some(r) = rect {
                    state.scene.set_crop(Some(r));
                }
                state.invalidate_overlay();
                return UpdateOutcome::None;
            }
            let drop = match state.scene.in_progress() {
                Some(
                    Annotation::Rectangle { rect, .. }
                    | Annotation::Ellipse { rect, .. }
                    | Annotation::Pixelate { rect, .. },
                ) => rect.is_degenerate(),
                Some(
                    Annotation::Line { from, to, .. } | Annotation::Arrow { from, to, .. },
                ) => (from.x - to.x).abs() < 0.5 && (from.y - to.y).abs() < 0.5,
                Some(Annotation::Pen { points, .. }) => points.len() < 2,
                _ => false,
            };
            if drop {
                state.scene.cancel_in_progress();
            } else {
                state.scene.commit_in_progress();
            }
            state.invalidate_overlay();
            UpdateOutcome::None
        }
        Msg::TextEditChanged(t) => {
            if let Some(te) = state.text_edit.as_mut() {
                te.text = t;
            }
            UpdateOutcome::None
        }
        Msg::TextEditSubmit => {
            if let Some(te) = state.text_edit.take() {
                if !te.text.is_empty() {
                    state.scene.begin(Annotation::Text {
                        position: te.position,
                        content: te.text,
                        font_size: state.tools.text_size,
                        color: state.tools.color,
                    });
                    state.scene.commit_in_progress();
                    state.invalidate_overlay();
                }
            }
            UpdateOutcome::None
        }
        Msg::TextEditCancel => {
            state.text_edit = None;
            UpdateOutcome::None
        }
        Msg::SetColor(c) => {
            state.tools.color = c;
            UpdateOutcome::None
        }
        Msg::SetStrokeWidth(w) => {
            state.tools.stroke_width = w.clamp(1.0, 32.0);
            UpdateOutcome::None
        }
        Msg::SetTextSize(s) => {
            state.tools.text_size = s.max(6.0);
            UpdateOutcome::None
        }
        Msg::SetTileSize(t) => {
            state.tools.tile_size = t.max(4);
            UpdateOutcome::None
        }
    }
}

const PALETTE: &[Color] = &[
    Color::from_rgb(1.0, 0.0, 0.0),  // red
    Color::from_rgb(1.0, 0.5, 0.0),  // orange
    Color::from_rgb(1.0, 1.0, 0.0),  // yellow
    Color::from_rgb(0.0, 0.7, 0.0),  // green
    Color::from_rgb(0.0, 0.4, 1.0),  // blue
    Color::from_rgb(0.6, 0.0, 0.8),  // purple
    Color::from_rgb(0.0, 0.0, 0.0),  // black
    Color::from_rgb(1.0, 1.0, 1.0),  // white
];

fn color_eq(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 1e-3
        && (a.g - b.g).abs() < 1e-3
        && (a.b - b.b).abs() < 1e-3
        && (a.a - b.a).abs() < 1e-3
}

fn color_swatch<'a>(c: Color, active: bool) -> Element<'a, Msg> {
    let bg = cosmic::iced_core::Background::Color(c);
    let border_w: f32 = if active { 2.0 } else { 1.0 };
    let inner = container(space::horizontal())
        .width(Length::Fixed(20.0))
        .height(Length::Fixed(20.0))
        .class(cosmic::theme::Container::Custom(Box::new(move |_theme| {
            cosmic::iced::widget::container::Style {
                background: Some(bg),
                border: cosmic::iced_core::Border {
                    width: border_w,
                    color: Color::from_rgb(0.5, 0.5, 0.5),
                    radius: 4.0.into(),
                },
                ..Default::default()
            }
        })));
    button::custom(inner).on_press(Msg::SetColor(c)).into()
}

fn stepper<'a>(label: String, value: String, on_dec: Msg, on_inc: Msg) -> Element<'a, Msg> {
    row::with_children(vec![
        text(label).into(),
        button::standard("-").on_press(on_dec).into(),
        text(value).into(),
        button::standard("+").on_press(on_inc).into(),
    ])
    .spacing(4)
    .into()
}
