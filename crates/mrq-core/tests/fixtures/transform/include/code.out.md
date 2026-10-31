# Code

```rust
//! Hello.

pub mod greet {
    /// Greets.
    pub fn hello(name: &str) -> String {
        format!("Hello, {name}!")
    }
}
```

Lines 5 to 7, dedented by default:

```rust
pub fn hello(name: &str) -> String {
    format!("Hello, {name}!")
}
```

```rust
    pub fn hello(name: &str) -> String {
        format!("Hello, {name}!")
    }
```

```rust
//! Hello.
```

```rust
        format!("Hello, {name}!")
    }
}
```

```rust
//! Hello.

pub mod greet {
    /// Greets.
    pub fn hello(name: &str) -> String {
        format!("Hello, {name}!")
    }
}
```

Markdown with fences, as code:

`````markdown
# Fences

```sh
run
```

````markdown
```sh
nested
```
````

~~~
tildes
~~~
`````

> In a quote:
>
> ```rust
> format!("Hello, {name}!")
> ```

- In a list:

  ```rust
  pub mod greet {
      /// Greets.
      pub fn hello(name: &str) -> String {
  ```

This file, as code, is not a cycle:

```markdown
# Code

<!-- mrq:include path="code/hello.rs" lang=rust -->
```
