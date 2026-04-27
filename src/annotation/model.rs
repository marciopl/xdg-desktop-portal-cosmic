#![allow(dead_code, unused_variables)]
// Implemented in Task 2.

pub use cosmic::iced::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point { pub x: f32, pub y: f32 }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size { pub w: f32, pub h: f32 }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalRect { pub origin: Point, pub size: Size }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke { pub width: f32, pub color: Color }

#[derive(Clone, Debug)]
pub enum Annotation {}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Tool { #[default] Pen }

#[derive(Clone, Debug, Default)]
pub struct ToolState;

#[derive(Clone, Debug, Default)]
pub struct AnnotationScene;
