---
title: PDF fixture
---

# The PDF backend

A paragraph with *emphasis*, **strong text**, ~~strikethrough~~, `inline code`, a [link](https://example.com/docs "Docs") and an autolink <https://example.org>. Typst's markup characters stay literal: # $ * _ ` < > @ [ ] / \ = - + ~ "quotes" and
= at a line start.\
After a hard break, a footnote[^note] and the same one again[^note]. See [the table](#tables).

## Lists

- First item
- Second item
  - Nested item
- [ ] An open task
- [x] A done task

3. Three
4. Four

## Quotes and alerts

> A blockquote with *emphasis*.

> [!WARNING]
> Critical content demanding attention.

## Code

```rust
fn main() {
    println!("Hello, {}!", "world");
}
```

```
plain code
    indented line
```

```fake
graph
```

## Tables

| Left | Center | Right |
| :--- | :----: | ----: |
| alpha | `beta` | 1 |
| *gamma* | delta | 22 |

## Math

Inline $x^2 + y_1 = \frac{a}{b}$ and display:

$$
\sum_{i=1}^{n} \alpha_i \le \sqrt{z}
$$

## Images

![The logo](logo.svg)

***

[^note]: The footnote text with a [link](https://example.net).
