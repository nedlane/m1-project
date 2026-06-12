# m1-project

A CLI that makes **structured, validated edits** to a MoTeC M1
`Project.m1prj` — create channels, parameters, tables, groups and scripts,
change a component's permissions / unit / type, set a script's execution rate
— **from the editor** instead of hand-editing a large XML file and guessing
the conventions.

It is the write-side companion to the read-only
[m1-lsp](https://github.com/C-Nucifora/m1-lsp) language server: the LSP keeps
serving reads (hover, go-to-def, diagnostics), and `m1-project` owns the
mutations. The [m1-vscode](https://github.com/nedlane/m1-vscode) extension and
the [nvim-m1](https://github.com/C-Nucifora/nvim-m1) plugin both invoke this
binary, so the same edits are available in either editor.

## Why a separate tool

The `.m1prj` is a large XML file that **MoTeC M1-Build also writes**. To stay
out of its way, every edit here is **surgical**: the target element is located
byte-accurately and the smallest possible text change is spliced in, leaving
the rest of the file — formatting, comments, attribute order — untouched. And
every edit writes **exactly the shape M1-Build itself writes** (the same
element bodies, number formats, and view-tree bookkeeping), so a project
edited here looks to M1-Build like one it edited itself.

Edits are validated (valid security levels, known types, no duplicate names,
the target clock exists), so an invalid project can't be produced.

## Install

Prebuilt binaries for Linux, macOS, and Windows are attached to each
[release](https://github.com/nedlane/m1-project/releases) — the editor
integrations fetch them automatically. Or build from source:

```sh
cargo install --git https://github.com/nedlane/m1-project.git --tag <latest>
```

## Usage

```sh
m1-project create-channel --project Project.m1prj --name "Root.Driver.Throttle" --type f32 --unit %
m1-project set-call-rate  --project Project.m1prj --script "Root.Engine.Control" --rate 100
m1-project rename-component --project Project.m1prj --name "Root.Old" --new-name "New"
m1-project validate       --project Project.m1prj          # read-only structural check
m1-project list-components --project Project.m1prj --json  # machine-readable inventory
```

The verbs fall into four groups — run `m1-project --help` for the full list
and flags:

- **`create-*`** — channels, parameters, constants, tables, references,
  groups, and (scheduled) functions, including the backing `.m1scr` where
  M1-Build would create one.
- **`set-*` / `add-tag` / `remove-tag`** — the M1-Build *Properties* rows:
  security, type, unit, physical quantity, validation bounds, display
  format, tags, comments, and script call rate.
- **`delete-component` / `rename-component`** — structure edits with the
  bookkeeping M1-Build does (rename rewrites triggers that resolve into the
  subtree and renames the backing `.m1scr` on disk; delete refuses to orphan
  referencing scripts unless `--force`).
- **`validate` / `list-components` / `list-rates`** — read-only queries.
  `validate` mirrors M1-Build's own structural findings (referencing its
  error numbers), so CI can catch what M1-Build would flag before the project
  ever opens there.

Global flags: `--dry-run` (print the result, write nothing) and `--stdout`;
without either, the file is edited in place (atomically). JSON output is
available where it makes sense for scripting.

## Development

The CI gate is `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --all -- --check`. Corpus tests run against a real project checkout
when one is present as a sibling, and skip otherwise. Checks that need
cross-script dataflow or a unit-dimension model deliberately live in
[m1-typecheck](https://github.com/C-Nucifora/m1-typecheck), not here.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
