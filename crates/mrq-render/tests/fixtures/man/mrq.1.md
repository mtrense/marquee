# mrq(1) -- a Markdown toolkit for the terminal

## Synopsis

`mrq` `validate` [*options*] [*path*...]
`mrq` `render` `-o` *format* [*path*...]

## Description

**mrq** verifies, lints, formats and *renders* Markdown, with `code`, ***both*** and
`--dashes`. Backslashes \\ and dashes - are escaped, so are the
.leading dot and the
'leading quote. It's fine.\
After a hard break, a line can start
.with a dot too.

See [the README](https://github.com/mtrense/marquee) or <https://example.com>,
[a section](#options) and [another page](mrq-render.1.md).
An image: ![the logo](logo.png). A footnote[^note] and another[^other].

<div>raw HTML is dropped</div>

## Options

* `-o`, `--output` *format*:
  The output format.
* A loose item.

  With a second paragraph.

  * Nested
  * Items

1. First
2. Second
   1. Inner

- [ ] open task
- [x] done task

### Examples

```sh
mrq render -o man mrq.1.md > mrq.1
.not a request
'not one either
```

    indented code \fB stays literal

> A quote with **bold** text.

> [!WARNING]
> Mind the gap.

| Format | Notes |
| :----- | ----: |
| `man`  | roff, for *man* pages |
| `html` |       |

---

#### A minor heading

Done.

[^note]: The note, with `code`.
[^other]: Another note.

    With a second paragraph.

        and a code block
