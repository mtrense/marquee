# Rust

The fence language comes from the file extension:

```rust
fn main() {
    println!("{}", Point::new());
}
```

```rust
pub struct Point {
    pub x: f64,
    pub y: f64,
}
```

```rust
/// A point in the plane.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
```

A method, dedented, and `lines` counted from the symbol:

```rust
pub fn norm(&self) -> f64 {
    self.x.hypot(self.y)
}
```

```rs
Self { x: 0.0, y: 0.0 }
```

```rust
    /// The origin.
    pub fn new() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
```

Any kind of symbol, trait methods, trait impls and modules:

```rust
/// A shape.
pub enum Shape<T> {
    Circle(Point, T),
    Square(Point, T),
}
```

```rust
/// The area.
fn area(&self) -> f64;
```

```rust
fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "({}, {})", self.x, self.y)
}
```

```rust
#[inline]
pub fn clamp(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}
```

```rust
pub mod util {
    // Not a doc comment.
    #[inline]
    pub fn clamp(x: f64) -> f64 {
        x.clamp(0.0, 1.0)
    }
}
```

```rust
pub const ORIGIN: Point = Point { x: 0.0, y: 0.0 };
```

```rust
fn main() {
    println!("{}", Point::new());
}
```
