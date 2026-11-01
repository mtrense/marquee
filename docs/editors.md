# Editor setup for `mrq lsp`

`mrq lsp` is a Language Server Protocol server on stdio. Any editor with an LSP client can use it: start `mrq lsp` for Markdown files, with the workspace folder (or the directory holding `.mrq.yaml`) as the root. The global flags apply, e.g. `mrq --config path/to/.mrq.yaml lsp` or `mrq --flavor obsidian lsp`. See the [`mrq lsp` section of the README](../README.md#mrq-lsp) for what the server does.

What the server offers, and how editors usually reach it:

| Feature | LSP request | Neovim | Helix |
| ------- | ----------- | ------ | ----- |
| Diagnostics (`validate`, `lint`) | `textDocument/publishDiagnostics` | `]d`, `[d`, `<C-w>d` | `]d`, `[d`, `space d` |
| Go to definition | `textDocument/definition` | `<C-]>` | `gd` |
| References | `textDocument/references` | `grr` | `gr` |
| Hover | `textDocument/hover` | `K` | `space k` |
| Completion | `textDocument/completion` | `<C-x><C-o>` (or a completion plugin) | automatic |
| Rename a heading | `textDocument/rename` | `grn` | `space r` |
| Document and workspace symbols | `textDocument/documentSymbol`, `workspace/symbol` | `gO` | `space s`, `space S` |
| Format the document (`mrq fmt`) | `textDocument/formatting` | `:lua vim.lsp.buf.format()` | `:format` |
| Quick fixes, fix all, refresh includes | `textDocument/codeAction` | `gra` | `space a` |

Formatting follows the `fmt` section of the configuration of the document's root, exactly as `mrq fmt` does (including `fmt.code_formatters`); the editor's tab size and spaces settings are ignored. Only whole documents are formatted (no range formatting). The code actions are:

- a quick fix (`quickfix`) for each lint finding with a fix under the cursor or selection, or among the diagnostics the editor sends along;
- "Refresh include region" (`refactor.rewrite`) on an outdated `mrq:include` region, anywhere between its markers;
- "Fix all mrq lint findings" (`source.fixAll.mrq`): what `mrq lint --fix` writes;
- "Refresh all include regions" (`source.refreshIncludes.mrq`): what `mrq interpolate --in-place` writes, but offline (remote includes are not fetched).

## Neovim

Neovim 0.11 and later configure servers without plugins. In `init.lua` (or `after/lsp/mrq.lua` for the `vim.lsp.config` part):

```lua
vim.lsp.config('mrq', {
  cmd = { 'mrq', 'lsp' },
  filetypes = { 'markdown' },
  root_markers = { '.mrq.yaml', '.git' },
})
vim.lsp.enable('mrq')
```

Optionally, format on save and a key for "fix all":

```lua
vim.api.nvim_create_autocmd('BufWritePre', {
  pattern = { '*.md', '*.markdown' },
  callback = function(args)
    vim.lsp.buf.format({ bufnr = args.buf, name = 'mrq' })
  end,
})
vim.keymap.set('n', '<leader>mf', function()
  vim.lsp.buf.code_action({ context = { only = { 'source.fixAll' } }, apply = true })
end, { desc = 'mrq: fix all lint findings' })
```

On Neovim 0.10, without `vim.lsp.config`, start the server from a `FileType` autocommand:

```lua
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'markdown',
  callback = function(args)
    vim.lsp.start({
      name = 'mrq',
      cmd = { 'mrq', 'lsp' },
      root_dir = vim.fs.root(args.buf, { '.mrq.yaml', '.git' }),
    })
  end,
})
```

`:checkhealth vim.lsp` shows whether the server is attached, and `:lua vim.lsp.buf.code_action()` lists the actions at the cursor.

## Helix

In `~/.config/helix/languages.toml` (or `.helix/languages.toml` in a project):

```toml
[language-server.mrq]
command = "mrq"
args = ["lsp"]

[[language]]
name = "markdown"
language-servers = ["mrq"]
roots = [".mrq.yaml"]
# Format on save with `mrq fmt`'s rules.
auto-format = true
```

`language-servers` replaces Helix's default for Markdown (marksman); list both (`["mrq", "marksman"]`) to keep it. `hx --health markdown` shows whether `mrq` is found. `space a` lists the code actions at the selection, and `:format` formats the document.

## Other editors

Any client works the same way: run `mrq lsp` for the `markdown` language over stdio. The server negotiates UTF-8 positions when the client offers them (else UTF-16), uses versioned workspace edits when the client supports them, and asks the client to watch Markdown and config files when it supports dynamic registration.
