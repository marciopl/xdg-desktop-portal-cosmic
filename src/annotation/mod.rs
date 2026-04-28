#![allow(dead_code, unused_variables)]

pub mod model;
pub mod render;
pub mod widget;

pub use model::Tool;
pub use render::composite_annotations;
pub use widget::{AnnotationView, Msg as WidgetMsg, UpdateOutcome};
