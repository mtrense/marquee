//! Hello.

pub mod greet {
    /// Greets.
    pub fn hello(name: &str) -> String {
        format!("Hello, {name}!")
    }
}
