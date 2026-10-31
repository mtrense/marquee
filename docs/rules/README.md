# Lint rules

`mrq lint` runs markdownlint-compatible rules (`MD001`–`MD059`, same ids, aliases, tags, options and defaults as [markdownlint](https://github.com/DavidAnson/markdownlint/blob/main/doc/Rules.md)) plus mrq's own rules, documented here. mrq rules use what markdownlint does not know about: the document's flavor and its anchors, frontmatter values and `mrq:` directives.

Rules are configured under `lint.rules` in `.mrq.yaml` by id, alias or tag, with `true`/`on`, `false`/`off` or an options object (which also turns the rule on). Every mrq rule has the tag `mrq`, so `mrq: false` turns them all off. Findings are warnings with the rule id as code, and `mrq-disable*` / `markdownlint-disable*` comments apply to them as to any rule.

| Rule | Alias | Tags | Default | Fix | Description |
| --- | --- | --- | --- | --- | --- |
| [MRQ001](MRQ001.md) | `valid-anchors` | `links`, `mrq` | off | no | Link to a missing anchor |
| [MRQ002](MRQ002.md) | `heading-punctuation` | `headings`, `mrq` | off | yes | Heading text ends with punctuation |
| [MRQ003](MRQ003.md) | `frontmatter-key-order` | `frontmatter`, `mrq` | off | YAML | Frontmatter keys out of order |
| [MRQ004](MRQ004.md) | `stale-include` | `directives`, `mrq` | on | no | `mrq:include` region out of date |
| [MRQ005](MRQ005.md) | `no-url-wikilinks` | `links`, `url`, `mrq` | on | yes | Bare URL in a wikilink |

MRQ001 and MRQ002 are off by default because MD051 and MD026, which are on, cover similar ground: turn on the mrq rule instead of the markdownlint one when you want its behaviour. MRQ003 does nothing until it is given an order.

```yaml
lint:
  rules:
    MD051: off
    MRQ001: on
    MD026: off
    heading-punctuation: { punctuation: ".,;:!?" }
    MRQ003: { order: [title, date], alphabetical: true }
```
