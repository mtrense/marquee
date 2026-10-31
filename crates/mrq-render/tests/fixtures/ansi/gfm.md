---
title: ANSI fixture
---

# The ANSI backend

A paragraph with *emphasis*, **strong text**, ~~strikethrough~~, `inline code`, a [link](https://example.com/docs "Docs") and an autolink <https://example.org>. It is long enough to wrap at sixty columns and again at one hundred columns, so the snapshots show the wrapping.\
After a hard break, a footnote[^note] and another one[^second], then the first again[^note].

## Lists

- First item
- Second item with a longer text that has to wrap onto a second line at sixty columns
  - Nested item
    - Deeper item
- [ ] An open task
- [x] A done task

3. Three
4. Four
   1. Nested ordered
10. Ten

Loose list:

- One

- Two

### Quotes and alerts

> A blockquote with *emphasis*.
>
> > A nested quote.

> [!NOTE]
> Useful information that users should know, even when skimming content.

> [!WARNING]
> Critical content demanding immediate user attention due to potential risks.

#### Code

```rust
fn main() {
    println!("Hello, {}!", "world");
}
```

```
plain code
	with a tab
```

    indented code

```bob
+----+
| ok |
+----+
```

##### Table

| Left | Center | Right | Long column |
| :--- | :----: | ----: | ----------- |
| a | b | 1 | short |
| *emphasis* | `code` | 22 | a cell with enough text to make the table wider than sixty columns, which has to wrap |
| 漢字 | 😀 | 333 | wide characters |

###### Others

Term without a definition list, then a rule:

***

![An image that does not exist](missing.png)

<div class="raw">
  Raw <b>HTML</b>
</div>

[^note]: The footnote text with a [link](https://example.net).
[^second]: A second footnote.

    With a second paragraph.
[^unused]: Never referenced.
