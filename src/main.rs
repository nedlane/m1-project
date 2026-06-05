//! `m1-project` CLI: structured, validated edits to a MoTeC M1 `Project.m1prj`.
//!
//! Each subcommand reads the project, applies one surgical mutation, and writes it
//! back in place — unless `--dry-run` (print the result to stdout, don't write) or
//! `--stdout` (write to stdout instead of the file). Designed to be invoked by the
//! editor extensions (m1-vscode, nvim-m1) so a developer never hand-edits the XML.
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "m1-project",
    about = "Edit a MoTeC M1 Project.m1prj (create channels, set permissions/unit/type, set call rate)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Print the modified project to stdout instead of writing the file.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Write the result to stdout instead of back to the project file.
    #[arg(long, global = true)]
    stdout: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new BuiltIn.Channel under an existing group.
    CreateChannel {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Engine.NewSignal`.
        #[arg(long)]
        name: String,
        /// Storage type (f32, u16, bool, …, or an enum reference).
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,
        /// Display unit (e.g. `rpm`).
        #[arg(long)]
        unit: Option<String>,
        /// Security level (Tune, Calibration, Master Calibration, Resource).
        #[arg(long)]
        security: Option<String>,
    },
    /// Set a component's security / access level.
    SetSecurity {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        security: String,
    },
    /// Set a component's storage type.
    SetType {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long, value_name = "TYPE")]
        r#type: String,
    },
    /// Set a component's display unit.
    SetUnit {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        unit: String,
    },
    /// Set a script's execution rate (e.g. `100` Hz, or `startup`).
    SetCallRate {
        #[arg(long)]
        project: PathBuf,
        /// The script component, e.g. `Root.Engine.Update`.
        #[arg(long)]
        script: String,
        #[arg(long)]
        rate: String,
    },
    /// List the available execution rates (On <N>Hz clocks) in the project.
    ListRates {
        #[arg(long)]
        project: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    use Command::*;
    // list-rates is read-only; handle it before the read/edit/write flow.
    if let ListRates { project } = &cli.command {
        // Decode tolerantly: MoTeC writes Windows-1252 for non-ASCII bytes
        // (e.g. `°`), which `read_to_string` would reject as invalid UTF-8.
        let (xml, _enc) = m1_workspace::read_text_with_encoding(project)?;
        for r in m1_project::available_rates(&xml)? {
            println!("{r}");
        }
        return Ok(());
    }

    let project = match &cli.command {
        CreateChannel { project, .. }
        | SetSecurity { project, .. }
        | SetType { project, .. }
        | SetUnit { project, .. }
        | SetCallRate { project, .. } => project,
        ListRates { .. } => unreachable!(),
    };
    // Decode tolerantly and remember the source encoding so the write-back
    // re-encodes in the same encoding (don't transcode a Windows-1252 file to
    // UTF-8 behind MoTeC's back).
    let (xml, encoding) = m1_workspace::read_text_with_encoding(project)?;

    let out = match &cli.command {
        CreateChannel {
            name,
            r#type,
            unit,
            security,
            ..
        } => m1_project::create_channel(
            &xml,
            name,
            r#type.as_deref(),
            unit.as_deref(),
            security.as_deref(),
        )?,
        SetSecurity {
            component,
            security,
            ..
        } => m1_project::set_security(&xml, component, security)?,
        SetType {
            component, r#type, ..
        } => m1_project::set_type(&xml, component, r#type)?,
        SetUnit {
            component, unit, ..
        } => m1_project::set_unit(&xml, component, unit)?,
        SetCallRate { script, rate, .. } => m1_project::set_call_rate(&xml, script, rate)?,
        ListRates { .. } => unreachable!(),
    };

    if cli.dry_run || cli.stdout {
        print!("{out}");
    } else {
        // Defense in depth: never write XML that isn't well-formed. The surgical
        // edits are parser-located and validated by tests, but re-parsing the
        // result before the irreversible write guarantees a bug can never persist
        // corruption to the canonical project file (#5).
        if let Err(e) = roxmltree::Document::parse(&out) {
            return Err(format!(
                "refusing to write malformed XML to {}: {e}",
                project.display()
            )
            .into());
        }
        // Re-encode in the file's original encoding so a Windows-1252 `°` stays a
        // single 0xB0 byte rather than UTF-8 `0xC2 0xB0`. `encode_checked` refuses
        // (rather than silently writing `?`) when the new content needs a
        // character Windows-1252 cannot represent, e.g. an ohm `Ω` (m1-workspace#6).
        let bytes = m1_workspace::encode_checked(&out, encoding)
            .map_err(|e| format!("cannot save in the file's {encoding:?} encoding: {e}"))?;
        // Atomic write: a temp file in the same directory, fsync'd, then renamed
        // over the target — an interruption/panic/ENOSPC can no longer truncate
        // the irreplaceable project file mid-write (#6).
        write_atomic(project, &bytes)?;
        eprintln!("Updated {}", project.display());
    }
    Ok(())
}

/// Write `bytes` to `path` atomically: write a sibling temp file, `fsync` it,
/// then `rename` it over `path` (atomic on the same filesystem). Avoids the
/// `O_TRUNC`-then-write window that could leave the project file empty/partial.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Project.m1prj");
    // Same directory (so rename is atomic), hidden, and pid-tagged to avoid
    // colliding with a concurrent run.
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}
