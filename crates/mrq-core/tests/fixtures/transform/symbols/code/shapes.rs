//! Shapes.

use std::fmt;

/// A point in the plane.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    /// The origin.
    pub fn new() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    pub fn norm(&self) -> f64 {
        self.x.hypot(self.y)
    }
}

/// A shape.
pub enum Shape<T> {
    Circle(Point, T),
    Square(Point, T),
}

impl<T: From<f64>> Shape<T> {
    pub fn new() -> Self {
        Shape::Circle(Point::new(), T::from(1.0))
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

pub trait Area {
    /// The area.
    fn area(&self) -> f64;
}

pub mod util {
    // Not a doc comment.
    #[inline]
    pub fn clamp(x: f64) -> f64 {
        x.clamp(0.0, 1.0)
    }
}

pub const ORIGIN: Point = Point { x: 0.0, y: 0.0 };

/// Docs, then a blank line: not the docs of `main`.

fn main() {
    println!("{}", Point::new());
}
