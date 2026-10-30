# Spec fixtures

Vendored, unmodified copies of the Markdown specifications whose examples `tests/parse.rs` parses:

| File | Source | Version | Licence |
| --- | --- | --- | --- |
| `commonmark-spec.txt` | [commonmark/commonmark-spec](https://github.com/commonmark/commonmark-spec/blob/0.31.2/spec.txt) | 0.31.2 | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), © John MacFarlane |
| `gfm-spec.txt` | [github/cmark-gfm](https://github.com/github/cmark-gfm/blob/master/test/spec.txt) | 0.29-gfm | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), © John MacFarlane and GitHub |

These files are test data only. They are not part of any published crate (`mrq-core` excludes them from its package) and are distributed under their own licence, not mrq's MIT/Apache-2.0.

Examples are fenced by 32 backticks followed by `example` (the GFM spec adds an extension name such as `example table`), with the Markdown and the expected HTML separated by a `.` line; `→` stands for a tab.
