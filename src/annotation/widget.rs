#![allow(dead_code, unused_variables)]

use cosmic::Element;
use cosmic::iced::widget::Stack;
use cosmic::iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use cosmic::iced::{Length, Pixels, Vector};
use cosmic::iced::widget::canvas;
use cosmic::widget::{button, column, container, icon, mouse_area, row, space, text};
use image::RgbaImage;

use crate::annotation::model::{
    Annotation, AnnotationScene, Color, LocalRect, Point, Size, Stroke, Tool, ToolState,
};
use crate::fl;

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const ZOOM_STEP: f32 = 1.25;

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
    /// Geometry cache for committed annotations. Cleared only when
    /// `scene.committed_version()` changes (commit / undo / redo / set_crop).
    committed_cache: canvas::Cache,
    /// Geometry cache for in-progress annotation + crop dim. Cleared on every
    /// scene-mutation message — these layers change on every PointerMove.
    overlay_cache: canvas::Cache,
    /// Snapshot of `scene.committed_version()` last time the committed cache
    /// was rebuilt. Mismatch triggers a rebuild.
    committed_version_cached: u64,
    pointer_down: Option<Point>,
    /// Pending text edit: position (canvas-local), current text buffer, focus id.
    pub text_edit: Option<TextEditState>,
    pub zoom: f32,
    /// Stable id for the canvas scrollable so `scrollable::scroll_to` can target
    /// it after a cursor-centered zoom step.
    pub scrollable_id: cosmic::widget::Id,
    /// Last reported absolute scroll offset of the canvas scrollable. Updated
    /// from `Msg::ScrollChanged`. Used together with `cursor_in_viewport` to
    /// keep the image-space point under the cursor stationary across Ctrl+wheel
    /// zoom steps.
    pub scroll_offset: AbsoluteOffset,
    /// Cursor position in viewport-local coords (i.e. relative to the
    /// scrollable's visible rectangle, NOT the canvas), or `None` when the
    /// cursor is outside the editor area. Updated from `Msg::CursorMovedInViewport`
    /// / `Msg::CursorLeftViewport` produced by the wrapping `mouse_area`.
    pub cursor_in_viewport: Option<cosmic::iced::Point>,
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
            committed_cache: canvas::Cache::default(),
            overlay_cache: canvas::Cache::default(),
            committed_version_cached: 0,
            pointer_down: None,
            text_edit: None,
            zoom: 1.0,
            scrollable_id: cosmic::widget::Id::unique(),
            scroll_offset: AbsoluteOffset { x: 0.0, y: 0.0 },
            cursor_in_viewport: None,
        }
    }

    /// Invalidate the canvas geometry caches. The overlay (in-progress + crop dim)
    /// is always cleared. The committed cache is only cleared when
    /// `scene.committed_version()` differs from the last snapshot — mid-drag
    /// updates to the in-progress annotation don't bump version, so the committed
    /// geometry is reused.
    pub fn invalidate_caches(&mut self) {
        let scene_version = self.scene.committed_version();
        if scene_version != self.committed_version_cached {
            self.committed_cache.clear();
            self.committed_version_cached = scene_version;
        }
        self.overlay_cache.clear();
    }

    pub fn set_zoom(&mut self, z: f32) {
        let clamped = z.clamp(MIN_ZOOM, MAX_ZOOM);
        if (clamped - self.zoom).abs() > f32::EPSILON {
            self.zoom = clamped;
            // Geometry caches are zoom-dependent (we apply frame.scale(zoom) inside
            // draw), so they must be cleared. invalidate_caches() also recomputes
            // the committed_version snapshot — the version hasn't changed here, so
            // we just clear both caches directly.
            self.committed_cache.clear();
            self.overlay_cache.clear();
        }
    }

    pub fn zoom_in(&mut self) {
        self.set_zoom(self.zoom * ZOOM_STEP);
    }

    pub fn zoom_out(&mut self) {
        self.set_zoom(self.zoom / ZOOM_STEP);
    }

    pub fn zoom_reset(&mut self) {
        self.set_zoom(1.0);
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
    PointerDown(Point),
    PointerMove(Point),
    PointerUp(Point),
    TextEditChanged(String),
    TextEditSubmit,
    TextEditCancel,
    SetColor(Color),
    SetStrokeWidth(f32),
    SetTextSize(f32),
    SetTileSize(u32),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// Cursor moved over the scrollable viewport. Coords are viewport-local
    /// (relative to the scrollable's visible rect).
    CursorMovedInViewport(cosmic::iced::Point),
    /// Cursor left the scrollable viewport.
    CursorLeftViewport,
    /// The scrollable's content has been scrolled (or its size changed).
    ScrollChanged(Viewport),
    /// Ctrl+wheel-up: zoom in while keeping the image-space point under the
    /// cursor stationary. Emitted only by the canvas wheel handler.
    ZoomAtCursorIn,
    /// Ctrl+wheel-down: zoom out while keeping the image-space point under
    /// the cursor stationary.
    ZoomAtCursorOut,
}

pub fn view(state: &AnnotationView) -> Element<'_, Msg> {
    let toolbar = build_toolbar(state);

    // Render the canvas at captured pixel size * zoom. Widget coords map to
    // canvas-local pixels at the zoomed scale; AnnotationProgram::update divides
    // cursor coords by zoom to recover image-space coordinates.
    let img_px_w = state.captured.width() as f32;
    let img_px_h = state.captured.height() as f32;
    let canvas_w = Length::Fixed(img_px_w * state.zoom);
    let canvas_h = Length::Fixed(img_px_h * state.zoom);

    let bg_image: Element<'_, Msg> = cosmic::widget::image(state.captured_handle.clone())
        .width(canvas_w)
        .height(canvas_h)
        .into();

    let canvas_overlay: Element<'_, Msg> = cosmic::widget::canvas(AnnotationProgram {
        captured: &state.captured,
        scene: &state.scene,
        committed_cache: &state.committed_cache,
        overlay_cache: &state.overlay_cache,
        zoom: state.zoom,
    })
    .width(canvas_w)
    .height(canvas_h)
    .into();

    // Anchor the Stack to top-left and let it shrink to its intrinsic Fixed size.
    // Without this `container` wrapper, when the surrounding scrollable's
    // content area is taller (or wider) than the canvas — i.e. when zoomed out
    // below 100% so the canvas is smaller than the viewport — the Stack's
    // children end up rendered at a different vertical position than where
    // `Layout::bounds()` reports the canvas widget to be. The cursor coords
    // returned by `cursor.position_in(bounds)` are computed relative to that
    // reported `bounds.y`, which is the top of the unscrolled scrollable
    // content area (i.e. the toolbar's bottom). The canvas bg_image and
    // committed/overlay geometry, however, get drawn from an origin that is
    // shifted by any layout slack. Wrapping in an explicit Length::Shrink
    // container with Start alignment pins the Stack to top-left so input and
    // render coordinate frames stay aligned regardless of viewport size.
    let canvas_layer: Element<'_, Msg> = container(
        Stack::with_children(vec![bg_image, canvas_overlay])
            .width(canvas_w)
            .height(canvas_h),
    )
    .width(Length::Shrink)
    .height(Length::Shrink)
    .align_x(cosmic::iced::alignment::Horizontal::Left)
    .align_y(cosmic::iced::alignment::Vertical::Top)
    .into();

    // If a text edit is in progress, layer a positioned text_input over the canvas.
    let canvas_element: Element<'_, Msg> = if let Some(te) = &state.text_edit {
        // Text-edit position is stored in image-space; scale into widget-space.
        let leading_x = (te.position.x * state.zoom).max(0.0);
        let leading_y = (te.position.y * state.zoom).max(0.0);
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
        // Same pinning treatment as the bg_image+canvas Stack above so the
        // text-edit overlay tracks the canvas in image-space coordinates even
        // when the scrollable's viewport is taller/wider than the canvas.
        container(
            Stack::with_children(vec![canvas_layer, positioned])
                .width(canvas_w)
                .height(canvas_h),
        )
        .width(Length::Shrink)
        .height(Length::Shrink)
        .align_x(cosmic::iced::alignment::Horizontal::Left)
        .align_y(cosmic::iced::alignment::Vertical::Top)
        .into()
    } else {
        canvas_layer
    };

    // Wrap the canvas in a scrollable so users can pan when zoomed past the window.
    // `id` + `on_scroll` let us track the scroll offset so cursor-centered Ctrl+wheel
    // zoom can compute a new offset that keeps the image point under the cursor.
    let scrollable_canvas: Element<'_, Msg> = cosmic::widget::scrollable(canvas_element)
        .id(state.scrollable_id.clone())
        .on_scroll(Msg::ScrollChanged)
        .direction(Direction::Both {
            vertical: Scrollbar::default(),
            horizontal: Scrollbar::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

    // The mouse_area wrapping the scrollable reports cursor coords that are
    // local to the *viewport* (visible rect of the scrollable), which is what
    // cursor-centered zoom needs. The canvas's own update handler only sees
    // canvas-local coords (which include scroll), so we can't derive viewport
    // coords cleanly there.
    let tracked_scrollable: Element<'_, Msg> = mouse_area(scrollable_canvas)
        .on_move(Msg::CursorMovedInViewport)
        .on_exit(Msg::CursorLeftViewport)
        .into();

    column::with_children(vec![toolbar, tracked_scrollable])
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

    let zoom_pct = (state.zoom * 100.0).round() as i32;
    let zoom_label: Element<'_, Msg> = container(text(format!("{zoom_pct}%")))
        .center_x(Length::Fixed(48.0))
        .into();
    let zoom_group = row::with_children(vec![
        icon_action_btn(
            "zoom-out-symbolic",
            fl!("tool-zoom-out"),
            "Ctrl+-",
            Msg::ZoomOut,
        ),
        zoom_label,
        icon_action_btn(
            "zoom-in-symbolic",
            fl!("tool-zoom-in"),
            "Ctrl++",
            Msg::ZoomIn,
        ),
        icon_action_btn(
            "zoom-original-symbolic",
            fl!("tool-zoom-reset"),
            "Ctrl+0",
            Msg::ZoomReset,
        ),
    ])
    .spacing(4);

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

    let inner = column::with_children(vec![
        tools.into(),
        row::with_children(vec![
            palette_row.into(),
            stroke,
            tool_specific,
            space::horizontal().width(Length::Fill).into(),
            zoom_group.into(),
            history_and_exit.into(),
        ])
        .spacing(16)
        .into(),
    ])
    .spacing(8)
    .padding(8);

    // Translucent backdrop so the toolbar stays legible over light captures.
    container(inner)
        .class(cosmic::theme::Container::Custom(Box::new(|theme| {
            let palette = theme.cosmic();
            let mut bg: cosmic::iced::Color = palette.background.component.base.into();
            bg.a = 0.85;
            cosmic::iced::widget::container::Style {
                background: Some(cosmic::iced::Background::Color(bg)),
                text_color: Some(palette.background.component.on.into()),
                ..Default::default()
            }
        })))
        .into()
}

pub enum UpdateOutcome {
    None,
    Done,
    Cancel,
    /// Caller should issue `cosmic::iced::widget::scrollable::scroll_to(id, offset)`.
    /// Used after a cursor-centered zoom step to reposition the canvas so the
    /// image-space point under the cursor stays put.
    ScrollTo(cosmic::widget::Id, AbsoluteOffset),
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
            state.invalidate_caches();
            UpdateOutcome::None
        }
        Msg::Redo => {
            state.scene.redo();
            state.invalidate_caches();
            UpdateOutcome::None
        }
        Msg::ResetCrop => {
            state.scene.set_crop(None);
            state.invalidate_caches();
            UpdateOutcome::None
        }
        Msg::PointerDown(cp) => {
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
            state.invalidate_caches();
            UpdateOutcome::None
        }
        Msg::PointerMove(cp) => {
            let Some(start) = state.pointer_down else {
                return UpdateOutcome::None;
            };
            match state.tools.active_tool {
                Tool::Pen => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Pen { points, .. } = a {
                            points.push(cp);
                        }
                    });
                }
                Tool::Line => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Line { to, .. } = a {
                            *to = cp;
                        }
                    });
                }
                Tool::Arrow => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Arrow { to, .. } = a {
                            *to = cp;
                        }
                    });
                }
                Tool::Rectangle => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Rectangle { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                }
                Tool::Ellipse => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Ellipse { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                }
                Tool::Pixelate => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Pixelate { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                }
                Tool::Crop => {
                    state.scene.update_in_progress(|a| {
                        if let Annotation::Rectangle { rect, .. } = a {
                            *rect = LocalRect::from_corners(start, cp);
                        }
                    });
                }
                Tool::Text => {
                    return UpdateOutcome::None;
                }
            }
            state.invalidate_caches();
            UpdateOutcome::None
        }
        Msg::PointerUp(_cp) => {
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
                state.invalidate_caches();
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
            state.invalidate_caches();
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
                    state.invalidate_caches();
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
        Msg::ZoomIn => {
            state.zoom_in();
            UpdateOutcome::None
        }
        Msg::ZoomOut => {
            state.zoom_out();
            UpdateOutcome::None
        }
        Msg::ZoomReset => {
            state.zoom_reset();
            UpdateOutcome::None
        }
        Msg::CursorMovedInViewport(p) => {
            state.cursor_in_viewport = Some(p);
            UpdateOutcome::None
        }
        Msg::CursorLeftViewport => {
            state.cursor_in_viewport = None;
            UpdateOutcome::None
        }
        Msg::ScrollChanged(viewport) => {
            state.scroll_offset = viewport.absolute_offset();
            UpdateOutcome::None
        }
        Msg::ZoomAtCursorIn => zoom_at_cursor(state, ZoomDir::In),
        Msg::ZoomAtCursorOut => zoom_at_cursor(state, ZoomDir::Out),
    }
}

enum ZoomDir {
    In,
    Out,
}

/// Apply a single zoom step at the cursor. After the zoom we want the image-space
/// point that was under the cursor at zoom `z0` to remain under it at zoom `z1`.
///
/// Image-space point under cursor: `img = (cursor + scroll) / z0`.
/// New scroll required: `scroll' = img * z1 - cursor = ((cursor + scroll) / z0) * z1 - cursor`.
/// Clamped to non-negative because the scrollable doesn't accept negative offsets.
fn zoom_at_cursor(state: &mut AnnotationView, dir: ZoomDir) -> UpdateOutcome {
    let z0 = state.zoom;
    match dir {
        ZoomDir::In => state.zoom_in(),
        ZoomDir::Out => state.zoom_out(),
    }
    let z1 = state.zoom;
    // No-op when already clamped at MIN_ZOOM/MAX_ZOOM — leave scroll alone.
    if (z1 - z0).abs() < f32::EPSILON {
        return UpdateOutcome::None;
    }
    let Some(cursor) = state.cursor_in_viewport else {
        // No known cursor (e.g. wheel event without prior mouse_area on_move)
        // — let the existing top-left-anchored behaviour stand.
        return UpdateOutcome::None;
    };
    // Defensive: z0 is clamped >= MIN_ZOOM so this is well-defined; the floor
    // mirrors AnnotationProgram::draw's divide-by-zero guard.
    let z0_safe = z0.max(1e-3);
    let sx = state.scroll_offset.x;
    let sy = state.scroll_offset.y;
    let new_sx = (((cursor.x + sx) / z0_safe) * z1 - cursor.x).max(0.0);
    let new_sy = (((cursor.y + sy) / z0_safe) * z1 - cursor.y).max(0.0);
    let new_offset = AbsoluteOffset { x: new_sx, y: new_sy };
    state.scroll_offset = new_offset;
    UpdateOutcome::ScrollTo(state.scrollable_id.clone(), new_offset)
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

// ============================================================================
// Canvas Program: live preview of committed + in-progress + crop dim.
// ============================================================================

struct AnnotationProgram<'a> {
    captured: &'a RgbaImage,
    scene: &'a AnnotationScene,
    committed_cache: &'a canvas::Cache,
    overlay_cache: &'a canvas::Cache,
    zoom: f32,
}

#[derive(Default)]
struct ProgramState {
    modifiers: cosmic::iced::keyboard::Modifiers,
    /// Accumulated wheel delta since the last zoom step. Lines are in "notches"
    /// (1.0 ≈ one wheel detent), Pixels are in raw scroll pixels (~120 per
    /// notch on most platforms). Once the magnitude crosses the threshold for
    /// its kind we emit one ZoomIn/ZoomOut and subtract the consumed amount.
    /// This stops high-resolution wheels and trackpads from firing many steps
    /// per detent.
    wheel_accum: f32,
    /// Tracks whether the last wheel delta was Pixels (true) or Lines (false)
    /// so the threshold matches the source units. Trackpad pixel events are
    /// ~120/notch; line events are 1/notch.
    wheel_is_pixels: bool,
}

/// One wheel-line ("notch") triggers one zoom step.
const WHEEL_LINE_STEP: f32 = 1.0;
/// Pixel scroll threshold per zoom step, matching the standard 120 pixels per
/// wheel notch convention used by libinput and most toolkits.
const WHEEL_PIXEL_STEP: f32 = 120.0;

// Parameterized over `cosmic::Theme` so the resulting `Canvas` produces a
// `cosmic::Element` (which is `Element<'_, M, cosmic::Theme, cosmic::Renderer>`).
// The default `Program` theme is `iced::Theme`, which would produce an iced
// Element that doesn't satisfy `cosmic::Element`'s trait bounds.
impl<'a> canvas::Program<Msg, cosmic::Theme> for AnnotationProgram<'a> {
    type State = ProgramState;

    fn update(
        &self,
        state: &mut ProgramState,
        event: &cosmic::iced::Event,
        bounds: cosmic::iced::Rectangle,
        cursor: cosmic::iced::mouse::Cursor,
    ) -> Option<canvas::Action<Msg>> {
        use cosmic::iced::{keyboard, mouse};

        // Track keyboard modifiers so wheel events know whether Ctrl is held.
        if let cosmic::iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            state.modifiers = *m;
            return None;
        }

        // Ctrl+wheel is handled BEFORE the `position_in(bounds)?` early-return
        // below: when the canvas is smaller than the surrounding scrollable
        // viewport (e.g. zoomed out on a small region), the cursor may be over
        // empty scrollable area rather than the canvas pixels. The canvas
        // widget itself still receives the wheel event from the scrollable, so
        // we must intercept it without requiring `cursor` to be inside
        // `bounds`. We also accumulate the delta to debounce high-resolution
        // wheels and trackpads, which can fire many small events per notch.
        if let cosmic::iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
            && state.modifiers.control()
        {
            let (dy, is_pixels) = match delta {
                mouse::ScrollDelta::Lines { y, .. } => (*y, false),
                mouse::ScrollDelta::Pixels { y, .. } => (*y, true),
            };
            if dy == 0.0 {
                return None;
            }
            // Reset the accumulator if the input source kind changed (Lines vs
            // Pixels) or the direction flipped — otherwise leftover delta from
            // the previous gesture would warp the next.
            if state.wheel_is_pixels != is_pixels
                || state.wheel_accum.signum() != dy.signum()
            {
                state.wheel_accum = 0.0;
                state.wheel_is_pixels = is_pixels;
            }
            state.wheel_accum += dy;
            let step = if is_pixels { WHEEL_PIXEL_STEP } else { WHEEL_LINE_STEP };
            if state.wheel_accum >= step {
                state.wheel_accum -= step;
                return Some(canvas::Action::publish(Msg::ZoomAtCursorIn).and_capture());
            } else if state.wheel_accum <= -step {
                state.wheel_accum += step;
                return Some(canvas::Action::publish(Msg::ZoomAtCursorOut).and_capture());
            }
            // Below threshold: capture the event so it doesn't fall through to
            // the scrollable's pan handling, but emit no zoom message yet.
            return Some(canvas::Action::<Msg>::capture());
        }

        let pos = cursor.position_in(bounds)?;
        // Descale widget-space cursor coords back into image-space so all
        // downstream geometry math (scene, draw_annotation) stays in image-space.
        // The zoom field is clamped >= MIN_ZOOM at the model layer; the 1e-3 floor
        // is a defensive guard against divide-by-zero only.
        let zoom = self.zoom.max(1e-3);
        let cp = Point { x: pos.x / zoom, y: pos.y / zoom };
        match event {
            cosmic::iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                Some(canvas::Action::publish(Msg::PointerDown(cp)).and_capture())
            }
            cosmic::iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                Some(canvas::Action::publish(Msg::PointerMove(cp)).and_capture())
            }
            cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(canvas::Action::publish(Msg::PointerUp(cp)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &ProgramState,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: cosmic::iced::Rectangle,
        _cursor: cosmic::iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // Caches are cleared on zoom change (set_zoom), so re-tessellation runs at
        // the new zoom. We apply a single frame-level scale so all downstream
        // geometry math (draw_annotation, draw_crop_dim) stays in image-space.
        let zoom = self.zoom;
        // The crop-dim mask is an even-odd outer rect that must cover the full
        // image-space canvas. After frame.scale(zoom), bounds (in widget pixels)
        // becomes (bounds / zoom) in image-space coordinates.
        let img_size = cosmic::iced::Size::new(bounds.width / zoom, bounds.height / zoom);
        let committed = self
            .committed_cache
            .draw(renderer, bounds.size(), |frame| {
                frame.scale(zoom);
                for ann in self.scene.iter_committed() {
                    draw_annotation(frame, self.captured, ann);
                }
            });
        let overlay = self
            .overlay_cache
            .draw(renderer, bounds.size(), |frame| {
                frame.scale(zoom);
                if let Some(ann) = self.scene.in_progress() {
                    draw_annotation(frame, self.captured, ann);
                }
                if let Some(crop) = self.scene.crop() {
                    draw_crop_dim(frame, crop, img_size);
                }
            });
        vec![committed, overlay]
    }
}

// ============================================================================
// Per-tool drawing: ports `render::render_one` from tiny-skia to canvas
// primitives. Math is identical to the saved-file composite path; only the
// API calls change. Sub-pixel anti-aliasing drift between the two renderers
// is acceptable per spec.
// ============================================================================

fn ip(p: Point) -> cosmic::iced::Point {
    cosmic::iced::Point::new(p.x, p.y)
}

fn isize_from(s: crate::annotation::model::Size) -> cosmic::iced::Size {
    cosmic::iced::Size::new(s.w, s.h)
}

fn draw_annotation(frame: &mut canvas::Frame, captured: &RgbaImage, ann: &Annotation) {
    use canvas::{LineCap, LineJoin, Path, Stroke as CStroke, Style};

    match ann {
        Annotation::Pen { points, stroke } => {
            if points.len() < 2 {
                return;
            }
            let path = Path::new(|b| {
                b.move_to(ip(points[0]));
                for p in &points[1..] {
                    b.line_to(ip(*p));
                }
            });
            frame.stroke(
                &path,
                CStroke {
                    style: Style::Solid(stroke.color),
                    width: stroke.width.max(0.5),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..CStroke::default()
                },
            );
        }
        Annotation::Line { from, to, stroke } => {
            let path = Path::line(ip(*from), ip(*to));
            frame.stroke(
                &path,
                CStroke {
                    style: Style::Solid(stroke.color),
                    width: stroke.width.max(0.5),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..CStroke::default()
                },
            );
        }
        Annotation::Arrow { from, to, stroke } => {
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.5 {
                return;
            }
            let head_len = (stroke.width * 4.0).max(8.0);
            let head_w = (stroke.width * 3.0).max(6.0);
            let ux = dx / len;
            let uy = dy / len;
            let bx = to.x - ux * head_len;
            let by = to.y - uy * head_len;
            let nx = -uy;
            let ny = ux;
            let p1 = (to.x, to.y);
            let p2 = (bx + nx * head_w * 0.5, by + ny * head_w * 0.5);
            let p3 = (bx - nx * head_w * 0.5, by - ny * head_w * 0.5);

            // Shaft: stop at the base of the head so it doesn't poke through the tip.
            let shaft = Path::line(ip(*from), cosmic::iced::Point::new(bx, by));
            frame.stroke(
                &shaft,
                CStroke {
                    style: Style::Solid(stroke.color),
                    width: stroke.width.max(0.5),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..CStroke::default()
                },
            );

            // Filled triangle head.
            let head = Path::new(|b| {
                b.move_to(cosmic::iced::Point::new(p1.0, p1.1));
                b.line_to(cosmic::iced::Point::new(p2.0, p2.1));
                b.line_to(cosmic::iced::Point::new(p3.0, p3.1));
                b.close();
            });
            frame.fill(&head, stroke.color);
        }
        Annotation::Rectangle { rect, stroke } => {
            if rect.is_degenerate() {
                return;
            }
            let path = Path::rectangle(ip(rect.origin), isize_from(rect.size));
            frame.stroke(
                &path,
                CStroke {
                    style: Style::Solid(stroke.color),
                    width: stroke.width.max(0.5),
                    ..CStroke::default()
                },
            );
        }
        Annotation::Ellipse { rect, stroke } => {
            if rect.is_degenerate() {
                return;
            }
            let cx = rect.origin.x + rect.size.w / 2.0;
            let cy = rect.origin.y + rect.size.h / 2.0;
            let rx = rect.size.w / 2.0;
            let ry = rect.size.h / 2.0;
            let path = Path::new(|b| {
                b.ellipse(canvas::path::arc::Elliptical {
                    center: cosmic::iced::Point::new(cx, cy),
                    radii: Vector::new(rx, ry),
                    rotation: cosmic::iced::Radians(0.0),
                    start_angle: cosmic::iced::Radians(0.0),
                    end_angle: cosmic::iced::Radians(2.0 * std::f32::consts::PI),
                });
            });
            frame.stroke(
                &path,
                CStroke {
                    style: Style::Solid(stroke.color),
                    width: stroke.width.max(0.5),
                    ..CStroke::default()
                },
            );
        }
        Annotation::Text { position, content, font_size, color } => {
            if content.is_empty() || *font_size <= 0.0 {
                return;
            }
            frame.fill_text(canvas::Text {
                content: content.clone(),
                position: ip(*position),
                color: *color,
                size: Pixels(*font_size),
                shaping: cosmic::iced_core::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
        }
        Annotation::Pixelate { rect, tile_size } => {
            draw_pixelate(frame, captured, rect, *tile_size);
        }
    }
}

fn draw_pixelate(
    frame: &mut canvas::Frame,
    captured: &RgbaImage,
    rect: &LocalRect,
    tile_size: u32,
) {
    if rect.is_degenerate() || tile_size == 0 {
        return;
    }
    let tw = captured.width() as i32;
    let th = captured.height() as i32;
    let x0 = (rect.origin.x.round() as i32).max(0);
    let y0 = (rect.origin.y.round() as i32).max(0);
    let x1 = ((rect.origin.x + rect.size.w).round() as i32).min(tw);
    let y1 = ((rect.origin.y + rect.size.h).round() as i32).min(th);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let ts = tile_size as i32;
    let src = captured.as_raw();
    let stride = captured.width() as i32 * 4;

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
                    let i = (py * stride + px * 4) as usize;
                    r_acc += src[i] as u64;
                    g_acc += src[i + 1] as u64;
                    b_acc += src[i + 2] as u64;
                    a_acc += src[i + 3] as u64;
                    count += 1;
                }
            }
            if count == 0 {
                tx += ts;
                continue;
            }
            let r = (r_acc / count) as u8;
            let g = (g_acc / count) as u8;
            let b = (b_acc / count) as u8;
            let a = (a_acc / count) as u8;
            let color = cosmic::iced::Color::from_rgba8(r, g, b, a as f32 / 255.0);
            frame.fill_rectangle(
                cosmic::iced::Point::new(tx as f32, ty as f32),
                cosmic::iced::Size::new((bx1 - tx) as f32, (by1 - ty) as f32),
                color,
            );
            tx += ts;
        }
        ty += ts;
    }
}

/// Even-odd fill: outer canvas-sized rect + inner crop rect. The fill rule
/// causes the interior of the crop to be excluded, leaving a dim frame around it.
fn draw_crop_dim(
    frame: &mut canvas::Frame,
    crop: &LocalRect,
    canvas_size: cosmic::iced::Size,
) {
    use canvas::{Fill, Path, Style, fill::Rule};

    let path = Path::new(|b| {
        b.rectangle(cosmic::iced::Point::new(0.0, 0.0), canvas_size);
        b.rectangle(
            ip(crop.origin),
            cosmic::iced::Size::new(crop.size.w.max(0.0), crop.size.h.max(0.0)),
        );
    });
    frame.fill(
        &path,
        Fill {
            style: Style::Solid(cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 128.0 / 255.0)),
            rule: Rule::EvenOdd,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn zoom_clamps_to_min_and_max() {
        let mut view = AnnotationView::new(RgbaImage::new(10, 10));
        view.set_zoom(0.0);
        assert!((view.zoom - MIN_ZOOM).abs() < 1e-6);
        view.set_zoom(100.0);
        assert!((view.zoom - MAX_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn zoom_in_out_round_trips_within_tolerance() {
        let mut view = AnnotationView::new(RgbaImage::new(10, 10));
        view.zoom_in();
        let after_in = view.zoom;
        view.zoom_out();
        assert!((view.zoom - 1.0).abs() < 1e-3, "round trip from 1.0 -> {after_in} -> {}", view.zoom);
    }

    #[test]
    fn zoom_reset_returns_to_one() {
        let mut view = AnnotationView::new(RgbaImage::new(10, 10));
        view.set_zoom(2.5);
        view.zoom_reset();
        assert!((view.zoom - 1.0).abs() < 1e-6);
    }
}
