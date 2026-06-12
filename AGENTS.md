# AGENTS.md — m1-project

Guidance for coding agents working in this repository.

## Purpose

The write side of the M1 toolchain: structured, validated edits to
`Project.m1prj`. The language server (`m1-lsp`) deliberately stays read-only;
every mutation editors offer goes through this binary. Both editor
integrations (m1-vscode, nvim-m1) shell out to it, so a verb's CLI contract
(flags, exit codes, JSON shapes) is a public API — changing it breaks editors.

## Things that are deliberate (don't "fix" them)

- **Surgical text splices, never re-serialisation.** The `.m1prj` is also
  written by MoTeC M1-Build. Parsing to a DOM and writing it back would churn
  formatting, attribute order, and encoding, producing unreviewable diffs and
  fighting M1-Build. Locate the target byte-accurately, splice the minimal
  change, leave everything else untouched.
- **M1-Build's output is the spec.** Every create/set verb writes exactly the
  shape M1-Build writes — element bodies, `%.17e` number form, CDATA comment
  layout, the dual `<List>`/`<Organisation>` bookkeeping. When adding a verb,
  diff what M1-Build itself produces for that edit (the real corpora are full
  of examples) and match it byte-for-byte; don't invent a cleaner shape.
- **Validation mirrors M1-Build's findings**, referencing its error numbers,
  and stays structural. Checks that need cross-script dataflow or a
  unit-dimension model belong in `m1-typecheck`, not here.
- **Real project files are Windows-1252 in practice.** All reads/writes go
  through `m1-workspace`'s tolerant decode and atomic-write helpers — never
  raw `fs::read_to_string` / in-place truncating writes.
- **Refuse rather than guess.** Unknown types/security levels, duplicate
  names, missing clocks, deletes that would orphan references — all hard
  errors. An invalid project must be unproducible.

## Build / test gate

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI also runs rustdoc with `-D warnings`, a security audit, and an MSRV job.
The MSRV pin in CI (`dtolnay/rust-toolchain@<version>`) must stay in sync with
`rust-version` in `Cargo.toml` — never bump one without the other.

Corpus tests need a real project as a sibling checkout and skip otherwise —
run them against a real corpus before trusting a serialisation change.

## Dependencies and releases

Depends on `m1-workspace` via a **versioned git tag** — never
`branch`/`path`/`[patch]`; the repo must build exactly like a public clone.
This is a binary repo: a version bump on `main` makes `release.yml` tag it
and upload prebuilt binaries, which the editor integrations fetch. After
releasing, open the editor pin-bump PRs immediately rather than waiting for
automation.
