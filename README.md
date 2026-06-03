# m1-project

A small CLI that makes **structured, validated edits** to a MoTeC M1
`Project.m1prj` — so a developer can create channels, change a component's
permissions / unit / type, and set a script's execution rate **from the editor**
instead of hand-editing a large XML file and guessing the conventions.

It is the write-side companion to the read-only [`m1-lsp`](https://github.com/C-Nucifora/m1-lsp)
language server: the LSP keeps serving reads (hover, go-to-def, diagnostics), and
`m1-project` owns the mutations. The [m1-vscode](https://github.com/nedlane/m1-vscode)
extension and the [nvim-m1](https://github.com/C-Nucifora/nvim-m1) plugin both invoke
this binary, so the same edits are available in either editor.

## Why a separate tool

The `.m1prj` is a large XML file that **MoTeC M1-Build also writes**. To stay out of
its way, every edit here is **surgical**: the target element is located byte-accurately
with `roxmltree` and the smallest possible text change is spliced in, leaving the rest
of the file — formatting, comments, attribute order — untouched. Edits are validated
(valid security levels, known types, no duplicate names, the target clock exists) so an
invalid project can't be produced.

## Commands

```
m1-project create-channel --project <Project.m1prj> --name <Root.Group.Name>
                          [--type f32] [--unit rpm] [--security Tune]
m1-project set-security   --project <p> --component <Root.X> --security <level>
m1-project set-type       --project <p> --component <Root.X> --type <type>
m1-project set-unit       --project <p> --component <Root.X> --unit <unit>
m1-project set-call-rate  --project <p> --script <Root.Group.Script> --rate <N|startup>
m1-project list-rates     --project <p>     # the On <N>Hz clocks available, one per line
```

Global flags: `--dry-run` (print the modified project to stdout, don't write) and
`--stdout` (write the result to stdout). Without either, the file is edited in place.

- **Security levels:** `Tune`, `Calibration`, `Master Calibration`, `Resource`.
- **Types:** `bool`, `u8`/`u16`/`u32`, `s8`/`s16`/`s32`, `f32`/`f64`, or an enum
  reference (`::This.Foo`, `MoTeC Types.Bar`).
- **Call rate:** `set-call-rate` writes the script's `<Props SelectedTrigger="…">`
  pointing at the matching `Root.Events.On <N>Hz` clock — the trigger is group-relative
  (`Parent.×N.Events.On <N>Hz`, one `Parent.` per path segment), exactly as M1-Build
  encodes it. `--rate startup` selects `On Startup`; the clock must already exist
  (`list-rates` shows what's available).

## Build

```sh
cargo build --release        # target/release/m1-project
cargo test                   # unit tests + (when the EV-M1 corpus is a sibling) corpus tests
```

Prebuilt binaries (Linux/macOS/Windows) are attached to each GitHub Release; the editor
extensions fetch them, so end users don't build from source.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
