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
        let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
            .map_err(|e| format!("{}: {e}", project.display()))?;
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
    // Decode tolerantly (UTF-8 with a Windows-1252 fallback). The write-back
    // encoding is determined from MoTeC's convention below, not by sniffing.
    let xml =
        m1_workspace::read_text(project).map_err(|e| format!("{}: {e}", project.display()))?;

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
        // Encode in the encoding MoTeC will READ the file back with — Windows-1252
        // by convention (the prolog omits `encoding=` and the doc declares a
        // `…1252` Locale) unless it explicitly declares UTF-8. Crucially this is
        // NOT the byte-sniffed encoding: a pure-ASCII project sniffs as UTF-8,
        // which would write a newly-inserted `°` as 2-byte UTF-8 that a 1252
        // reader mojibakes to `Â°` (#12). With 1252, `°` stays the single byte
        // 0xB0 and `encode_checked` REFUSES a unit MoTeC's 1252 cannot represent
        // (e.g. ohm `Ω`) rather than silently corrupting it.
        let encoding = motec_write_encoding(&out);
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

/// The encoding MoTeC will use to READ this XML back — which is what the
/// write-back must emit. MoTeC writes its project/config/CAN XML as
/// **Windows-1252** (the prolog omits `encoding=`, and the document declares a
/// `…1252` Locale), so 1252 is the default; only an explicit `encoding="utf-8"`
/// in the XML declaration means UTF-8. This deliberately does NOT use the
/// byte-sniffed encoding (`read_text_with_encoding`): a pure-ASCII project sniffs
/// as UTF-8, and a newly-inserted non-ASCII unit would then be written as UTF-8
/// that a 1252 reader mojibakes (#12).
fn motec_write_encoding(xml: &str) -> m1_workspace::Encoding {
    let head = &xml[..xml.len().min(256)];
    if let Some(end) = head.find("?>") {
        let decl = head[..end].to_ascii_lowercase();
        if decl.contains("encoding=\"utf-8\"") || decl.contains("encoding='utf-8'") {
            return m1_workspace::Encoding::Utf8;
        }
    }
    m1_workspace::Encoding::Windows1252
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motec_write_encoding_defaults_to_1252_not_sniffed_utf8() {
        // #12: MoTeC's prolog omits `encoding=`, so write-back must be
        // Windows-1252 (what MoTeC reads) — NOT the UTF-8 a pure-ASCII file
        // sniffs as. Only an explicit utf-8 declaration means UTF-8.
        assert_eq!(
            motec_write_encoding(
                "<?xml version=\"1.0\"?>\n<Project Locale=\"English_Australia.1252\"/>"
            ),
            m1_workspace::Encoding::Windows1252
        );
        assert_eq!(
            motec_write_encoding("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<x/>"),
            m1_workspace::Encoding::Utf8
        );
        assert_eq!(
            motec_write_encoding("<?xml version='1.0' encoding='utf-8'?>"),
            m1_workspace::Encoding::Utf8
        );
    }
}
