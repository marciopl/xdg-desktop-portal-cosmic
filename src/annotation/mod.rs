#![allow(dead_code, unused_variables)]

pub mod model;
pub mod render;
pub mod widget;

pub use model::{Annotation, AnnotationScene, Color, LocalRect, Point, Size, Stroke, Tool, ToolState};
pub use render::{composite_annotations, render_annotations};
pub use widget::{AnnotationView, Msg as WidgetMsg, UpdateOutcome};
