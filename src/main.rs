//! `m1-project` CLI: structured, validated edits to a MoTeC M1 `Project.m1prj`.
//!
//! Each subcommand reads the project, applies one surgical mutation, and writes it
//! back in place — unless `--dry-run` (print the result to stdout, don't write) or
//! `--stdout` (write to stdout instead of the file). Designed to be invoked by the
//! editor extensions (m1-vscode, nvim-m1) so a developer never hand-edits the XML.
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "m1-project",
    about = "Edit a MoTeC M1 Project.m1prj (create channels/groups, delete, rename, validate, list)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Preview: print a unified diff of what would change (and report skipped
    /// side effects like script renames) without touching any file.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Output routing: write the resulting XML to stdout instead of back to
    /// the project file (side effects like script renames are skipped).
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
    /// Create a new BuiltIn.GroupCompound under an existing group.
    CreateGroup {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Engine.NewSubsystem`.
        #[arg(long)]
        name: String,
    },
    /// Create a new BuiltIn.Parameter (an M1 Tune-tunable value) under a group.
    CreateParameter {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Engine.Gain`.
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
    /// Create a new BuiltIn.Constant (a fixed literal value) under a group.
    CreateConstant {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.CAN.CAN Bus Tertiary.Bus`.
        #[arg(long)]
        name: String,
        /// The literal value (e.g. `CAN Bus 1`).
        #[arg(long)]
        value: String,
    },
    /// Create a new BuiltIn.Table (1-3 axis lookup table) under a group.
    /// M1-Build generates the table's AutoCreated companions (.Value/.Init/
    /// .Update) when it next opens the project, as for a UI-created table.
    CreateTable {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Control.Pedal Map.Tune`.
        #[arg(long)]
        name: String,
        /// X-axis source channel (absolute `Root.…` path — validated and
        /// stored group-relative — or a `Parent.…` reference verbatim).
        #[arg(long, value_name = "SOURCE")]
        axis_x: String,
        /// Maximum X-axis sites (table breakpoints).
        #[arg(long, value_name = "N")]
        x_sites: Option<u32>,
        /// Y-axis source channel (makes the table 2-axis).
        #[arg(long, value_name = "SOURCE")]
        axis_y: Option<String>,
        /// Maximum Y-axis sites.
        #[arg(long, value_name = "N", requires = "axis_y")]
        y_sites: Option<u32>,
        /// Z-axis source channel (makes the table 3-axis).
        #[arg(long, value_name = "SOURCE", requires = "axis_y")]
        axis_z: Option<String>,
        /// Security level (Tune, Calibration, Master Calibration, Resource).
        #[arg(long)]
        security: Option<String>,
    },
    /// Create a new BuiltIn.Reference (an alias to a channel defined elsewhere).
    CreateReference {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Driver.Brake Pressure`.
        #[arg(long)]
        name: String,
        /// Optional explicit target (component-relative, e.g. `This.Value`);
        /// omitted for the usual name-implied reference.
        #[arg(long)]
        target: Option<String>,
    },
    /// Create a new BuiltIn.FuncUser scheduled function (creates its .m1scr too).
    CreateScheduledFunction {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Engine.Update`.
        #[arg(long)]
        name: String,
    },
    /// Create a new BuiltIn.FuncUserParam parametric function (creates its .m1scr too).
    CreateFunction {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified name, e.g. `Root.Engine.Compute`.
        #[arg(long)]
        name: String,
    },
    /// Delete a component (and optionally its whole subtree).
    DeleteComponent {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified component name to delete.
        #[arg(long)]
        name: String,
        /// Also delete all child components (the whole subtree).
        #[arg(long)]
        recursive: bool,
        /// Delete even if other components reference this one via SelectedTrigger.
        #[arg(long)]
        force: bool,
    },
    /// Rename a component, updating all SelectedTrigger references in the file.
    RenameComponent {
        #[arg(long)]
        project: PathBuf,
        /// Fully-qualified current name, e.g. `Root.Engine`.
        #[arg(long)]
        name: String,
        /// New single-segment name (no dots), e.g. `Motor`.
        #[arg(long)]
        new_name: String,
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
    /// Set a channel's storage class. Flash-backed values are committed only
    /// when project code calls System.Preserve(), at no more than 1 Hz.
    SetStorageClass {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        /// `flash` persists through System.Preserve(); `volatile` clears persistence.
        #[arg(long, value_parser = ["flash", "volatile"])]
        storage_class: String,
    },
    /// Change an existing BuiltIn.Constant's literal value (the M1-Build
    /// *Value* row, `<Props Value>`) — edits it in place, preserving the rest
    /// of the element (security, tags, comment), unlike delete-and-recreate.
    SetValue {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        /// The new literal value (e.g. `CAN Bus 2`).
        #[arg(long)]
        value: String,
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
    /// Set a component's physical quantity (`<Props Qty>`, e.g. `ratio`, `rad/s`).
    SetQuantity {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        quantity: String,
    },
    /// Set or clear a value component's validation bounds.
    SetValidation {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        /// Validation type: `MinMax` (needs --min/--max) or `None` (clears it).
        #[arg(long, value_name = "TYPE", default_value = "MinMax")]
        r#type: String,
        /// Lower bound (required for MinMax).
        #[arg(long, allow_hyphen_values = true)]
        min: Option<f64>,
        /// Upper bound (required for MinMax).
        #[arg(long, allow_hyphen_values = true)]
        max: Option<f64>,
    },
    /// Set a component's display format (`<Default Format>`, e.g. `Hex`, `Default`).
    SetFormat {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        format: String,
    },
    /// Set a component's decimal places (`<Default DPS>`).
    SetDps {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        dps: u32,
    },
    /// Set a component's display Min/Max (`<Default Min/Max>`; distinct from validation).
    SetDisplayRange {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long, allow_hyphen_values = true)]
        min: f64,
        #[arg(long, allow_hyphen_values = true)]
        max: f64,
    },
    /// Set or clear a component's comment (the *Comment* row; empty text clears it).
    SetComment {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        /// The comment text (stored as CDATA; may contain M1-Build rich-text HTML).
        #[arg(long, default_value = "")]
        comment: String,
    },
    /// Add a user tag to a component (the *Tags* row; fixes "Mandatory tag not selected").
    AddTag {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        tag: String,
    },
    /// Remove a user tag from a component.
    RemoveTag {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        tag: String,
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
    /// List the project's valid security groups (for a set-security picker).
    ListSecurity {
        #[arg(long)]
        project: PathBuf,
        /// Emit JSON (array of strings) instead of one group per line.
        #[arg(long)]
        json: bool,
    },
    /// Validate the project for structural correctness (read-only; exit 1 on findings).
    Validate {
        #[arg(long)]
        project: PathBuf,
        /// Directory containing the selected `.m1mod` files. May be repeated.
        /// Without this option, M1_MODULES_PATH and standard M1-Build install
        /// locations are searched. Module metadata enables inherited-tag checks.
        #[arg(long, value_name = "DIR")]
        modules_dir: Vec<PathBuf>,
        /// Emit JSON (array of objects with level/path/message) instead of text.
        #[arg(long)]
        json: bool,
        /// Also check known mandatory-Type-tag cases (M1-Build warning 1142):
        /// tables and assigned IO-resource parameters. Ordinary untagged channels
        /// and parameters are accepted. Warnings do not change the exit code.
        #[arg(long)]
        check_mandatory_tags: bool,
        /// Fail (error + exit 1) if the project's file format exceeds this version
        /// (e.g. `10108`). The gate that stops an accidental M1-Build upgrade from
        /// landing on `main`: a newer M1-Build silently migrates the format on open
        /// and then locks out every machine on an older build. Recover with
        /// `m1-project format --target <N>`.
        #[arg(long, value_name = "N")]
        max_format: Option<u32>,
    },
    /// List all components in the project.
    ListComponents {
        #[arg(long)]
        project: PathBuf,
        /// Emit JSON (array of objects with path/classname/type/unit/security/call_rate).
        #[arg(long)]
        json: bool,
    },
    /// Report the project's file-format version, or convert it with `--target`.
    ///
    /// Without `--target`, prints the current `FileFormat`, the M1-Build that last
    /// wrote it, the package target, and the known format→writer mappings. With
    /// `--target N`, rewrites the project to that format (only `10108 ↔ 10109` is
    /// supported) — a byte-exact, reversible conversion. Downgrade is the case that
    /// matters: it unblocks a teammate on an older M1-Build without everyone
    /// upgrading in lockstep. Honours the global `--dry-run` / `--stdout`.
    Format {
        #[arg(long)]
        project: PathBuf,
        /// Convert the project to this file format (e.g. `10108`). Omit to only
        /// report the current format.
        #[arg(long, value_name = "N")]
        target: Option<u32>,
    },
}

impl Command {
    /// The `--project` path this subcommand targets. Every subcommand carries one,
    /// so this is total — a `match` over all arms rather than an `unreachable!()`
    /// fallthrough, so adding a subcommand that *forgets* `project` is a compile
    /// error here instead of a silent runtime panic.
    fn project_path(&self) -> &PathBuf {
        match self {
            Command::CreateChannel { project, .. }
            | Command::CreateGroup { project, .. }
            | Command::CreateParameter { project, .. }
            | Command::CreateConstant { project, .. }
            | Command::CreateTable { project, .. }
            | Command::CreateReference { project, .. }
            | Command::CreateScheduledFunction { project, .. }
            | Command::CreateFunction { project, .. }
            | Command::DeleteComponent { project, .. }
            | Command::RenameComponent { project, .. }
            | Command::SetSecurity { project, .. }
            | Command::SetType { project, .. }
            | Command::SetStorageClass { project, .. }
            | Command::SetValue { project, .. }
            | Command::SetUnit { project, .. }
            | Command::SetQuantity { project, .. }
            | Command::SetComment { project, .. }
            | Command::SetValidation { project, .. }
            | Command::SetFormat { project, .. }
            | Command::SetDps { project, .. }
            | Command::SetDisplayRange { project, .. }
            | Command::AddTag { project, .. }
            | Command::RemoveTag { project, .. }
            | Command::SetCallRate { project, .. }
            | Command::ListRates { project, .. }
            | Command::ListSecurity { project, .. }
            | Command::Validate { project, .. }
            | Command::Format { project, .. }
            | Command::ListComponents { project, .. } => project,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use Command::*;

    // Read-only subcommands that don't go through the edit/write flow.
    match &cli.command {
        ListRates { project } => {
            // Decode tolerantly: MoTeC writes Windows-1252 for non-ASCII bytes
            // (e.g. `°`), which `read_to_string` would reject as invalid UTF-8.
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            for r in m1_project::available_rates(&xml)? {
                println!("{r}");
            }
            return Ok(ExitCode::SUCCESS);
        }
        ListSecurity { project, json } => {
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            let groups = m1_project::security_groups(&xml)?;
            if *json {
                let body = groups
                    .iter()
                    .map(|g| json_string(g))
                    .collect::<Vec<_>>()
                    .join(",");
                println!("[{body}]");
            } else {
                for g in &groups {
                    println!("{g}");
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        Validate {
            project,
            modules_dir,
            json,
            check_mandatory_tags,
            max_format,
        } => {
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            let module_xmls = selected_module_xmls(&xml, modules_dir)?;
            let module_refs: Vec<&str> = module_xmls.iter().map(String::as_str).collect();
            let mut findings = m1_project::validate_with_modules(&xml, &module_refs)?;
            // File-format gate (#): fail when the project's FileFormat exceeds the
            // team's pinned maximum — an accidental M1-Build upgrade the CI gate is
            // there to catch. A project-level finding (no component path), so it
            // uses the project file as its path.
            if let Some(maxf) = max_format
                && let Some(cur) = m1_project::file_format(&xml)
                && cur > *maxf
            {
                findings.push(m1_project::Finding {
                    level: m1_project::FindingLevel::Error,
                    path: project.display().to_string(),
                    message: format!(
                        "file format {cur} exceeds the maximum allowed {maxf} — a newer M1-Build has migrated this project; downgrade it with `m1-project format --target {maxf}`"
                    ),
                    code: None,
                });
            }
            // File-aware checks (only the CLI does I/O; `validate()` stays pure):
            //   - a script component whose backing `.m1scr` is missing/empty is
            //     M1-Build's "Missing code" error;
            //   - each DBCRoot module's backing `dbc/<module>.m1dbc` must exist and
            //     be internally consistent with the DBCRoot entry (#84).
            findings.extend(missing_code_findings(project, &xml));
            findings.extend(dbc_findings(project, &xml));
            // Opt-in known mandatory-Type-tag cases — warnings only, off by default.
            if *check_mandatory_tags {
                let existing_1648: std::collections::HashSet<String> = findings
                    .iter()
                    .filter(|finding| finding.code == Some(1648))
                    .map(|finding| finding.path.clone())
                    .collect();
                findings.extend(
                    m1_project::mandatory_tag_findings(&xml)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|finding| !existing_1648.contains(&finding.path)),
                );
            }
            findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.message.cmp(&b.message)));
            let errors = findings
                .iter()
                .filter(|f| f.level == m1_project::FindingLevel::Error)
                .count();
            let warnings = findings
                .iter()
                .filter(|f| f.level == m1_project::FindingLevel::Warning)
                .count();
            if *json {
                // One object per finding, machine-consumable (#42). Same
                // hand-rolled JSON helpers as list-components; exit semantics
                // unchanged (1 on errors).
                println!("[");
                for (i, f) in findings.iter().enumerate() {
                    let comma = if i + 1 < findings.len() { "," } else { "" };
                    let level = match f.level {
                        m1_project::FindingLevel::Error => "error",
                        m1_project::FindingLevel::Warning => "warning",
                    };
                    // The M1-Build error number, when known, as a bare JSON
                    // number (or `null`) — machine-readable so a CI consumer can
                    // triage/suppress by code without parsing the message (#83).
                    let code = match f.code {
                        Some(c) => c.to_string(),
                        None => "null".to_string(),
                    };
                    println!(
                        "  {{\"level\":{},\"path\":{},\"message\":{},\"code\":{}}}{}",
                        json_string(level),
                        json_string(&f.path),
                        json_string(&f.message),
                        code,
                        comma
                    );
                }
                println!("]");
            } else {
                for f in &findings {
                    println!("{f}");
                }
                println!(
                    "{} finding(s): {} error(s), {} warning(s)",
                    findings.len(),
                    errors,
                    warnings
                );
            }
            return Ok(if errors > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            });
        }
        ListComponents { project, json } => {
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            let entries = m1_project::list_components(&xml)?;
            if *json {
                println!("[");
                for (i, e) in entries.iter().enumerate() {
                    let comma = if i + 1 < entries.len() { "," } else { "" };
                    // Emit one JSON object per component.
                    let ty_json = json_string_or_null(e.ty.as_deref());
                    let unit_json = json_string_or_null(e.unit.as_deref());
                    let sec_json = json_string_or_null(e.security.as_deref());
                    let storage_json = json_string_or_null(e.storage_class.as_deref());
                    let cr_json = json_string_or_null(e.call_rate.as_deref());
                    let qty_json = json_string_or_null(e.qty.as_deref());
                    let tags_json = format!(
                        "[{}]",
                        e.tags
                            .iter()
                            .map(|t| json_string(t))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    let comment_json = json_string_or_null(e.comment.as_deref());
                    println!(
                        "  {{\"path\":{},\"classname\":{},\"type\":{},\"unit\":{},\"security\":{},\"storage_class\":{},\"call_rate\":{},\"qty\":{},\"tags\":{},\"comment\":{}}}{}",
                        json_string(&e.path),
                        json_string(&e.classname),
                        ty_json,
                        unit_json,
                        sec_json,
                        storage_json,
                        cr_json,
                        qty_json,
                        tags_json,
                        comment_json,
                        comma
                    );
                }
                println!("]");
            } else {
                for e in &entries {
                    let indent = "  ".repeat(e.depth);
                    let mut props = Vec::new();
                    if let Some(c) = &e.classname.strip_prefix("BuiltIn.") {
                        props.push(c.to_string());
                    } else {
                        props.push(e.classname.clone());
                    }
                    if let Some(t) = &e.ty {
                        props.push(format!("type={t}"));
                    }
                    if let Some(u) = &e.unit {
                        props.push(format!("unit={u}"));
                    }
                    if let Some(s) = &e.security {
                        props.push(format!("security={s}"));
                    }
                    if let Some(storage) = &e.storage_class {
                        props.push(format!("storage_class={storage}"));
                    }
                    if let Some(q) = &e.qty {
                        props.push(format!("qty={q}"));
                    }
                    if !e.tags.is_empty() {
                        props.push(format!("tags={}", e.tags.join("+")));
                    }
                    let segment = e.path.rsplit('.').next().unwrap_or(&e.path);
                    println!("{indent}{segment}  [{}]", props.join(", "));
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        Format { project, target } => {
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            match target {
                // Report only.
                None => {
                    let r = m1_project::format_report(&xml)?;
                    println!("{}", project.display());
                    println!(
                        "  FileFormat:      {}",
                        r.file_format
                            .map(|f| f.to_string())
                            .unwrap_or_else(|| "(none)".into())
                    );
                    let writer = match (&r.product_name, &r.product_version) {
                        (Some(n), Some(v)) => format!("{n} {v}"),
                        (None, Some(v)) => v.clone(),
                        _ => "(unknown)".into(),
                    };
                    println!("  Last written by: {writer}");
                    println!(
                        "  Package target:  {}",
                        r.package_target.as_deref().unwrap_or("(unknown)")
                    );
                    println!("  Known writers:   {}", m1_project::known_writers_summary());
                    return Ok(ExitCode::SUCCESS);
                }
                // Convert, honouring --dry-run / --stdout via the shared writer.
                Some(t) => {
                    let out = m1_project::convert_format(&xml, *t)?;
                    return write_or_print(cli, project, &xml, &out);
                }
            }
        }
        _ => {}
    }

    let project = cli.command.project_path();
    // Decode tolerantly (UTF-8 with a Windows-1252 fallback). The write-back
    // encoding is determined from MoTeC's convention below, not by sniffing.
    let xml =
        m1_workspace::read_text(project).map_err(|e| format!("{}: {e}", project.display()))?;

    // Subcommands that produce a warning (rename) are handled here before the
    // general edit/write flow.
    if let RenameComponent { name, new_name, .. } = &cli.command {
        let (out, script_renames) = m1_project::rename_component(&xml, name, new_name)?;
        // On a real write the backing .m1scr files are renamed FIRST (with
        // rollback on partial failure), and the XML only after every rename
        // succeeded (#49): a failure at any point leaves the old, loadable
        // project intact. --dry-run/--stdout leave the disk alone.
        if !cli.dry_run && !cli.stdout {
            let done = rename_script_files(project, &script_renames)?;
            return match write_or_print(cli, project, &xml, &out) {
                Ok(code) => Ok(code),
                Err(e) => {
                    rollback_renames(project, &done);
                    Err(e)
                }
            };
        }
        if cli.dry_run {
            for r in &script_renames {
                eprintln!("dry-run: would rename {} -> {}", r.old, r.new);
            }
        }
        return write_or_print(cli, project, &xml, &out);
    }

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
        CreateParameter {
            name,
            r#type,
            unit,
            security,
            ..
        } => m1_project::create_parameter(
            &xml,
            name,
            r#type.as_deref(),
            unit.as_deref(),
            security.as_deref(),
        )?,
        CreateConstant { name, value, .. } => m1_project::create_constant(&xml, name, value)?,
        CreateTable {
            name,
            axis_x,
            x_sites,
            axis_y,
            y_sites,
            axis_z,
            security,
            ..
        } => {
            let mut axes = vec![m1_project::TableAxis {
                source: axis_x.clone(),
                sites: *x_sites,
            }];
            if let Some(y) = axis_y {
                axes.push(m1_project::TableAxis {
                    source: y.clone(),
                    sites: *y_sites,
                });
            }
            if let Some(z) = axis_z {
                axes.push(m1_project::TableAxis {
                    source: z.clone(),
                    sites: None,
                });
            }
            m1_project::create_table(&xml, name, &axes, security.as_deref())?
        }
        CreateGroup { name, .. } => m1_project::create_group(&xml, name)?,
        CreateReference { name, target, .. } => {
            m1_project::create_reference(&xml, name, target.as_deref())?
        }
        CreateScheduledFunction { name, .. } => m1_project::create_scheduled_function(&xml, name)?,
        CreateFunction { name, .. } => m1_project::create_function(&xml, name)?,
        DeleteComponent {
            name,
            recursive,
            force,
            ..
        } => m1_project::delete_component(&xml, name, *recursive, *force)?,
        SetSecurity {
            component,
            security,
            ..
        } => m1_project::set_security(&xml, component, security)?,
        SetType {
            component, r#type, ..
        } => m1_project::set_type(&xml, component, r#type)?,
        SetStorageClass {
            component,
            storage_class,
            ..
        } => m1_project::set_storage_class(&xml, component, storage_class)?,
        SetValue {
            component, value, ..
        } => m1_project::set_value(&xml, component, value)?,
        SetUnit {
            component, unit, ..
        } => m1_project::set_unit(&xml, component, unit)?,
        SetQuantity {
            component,
            quantity,
            ..
        } => m1_project::set_quantity(&xml, component, quantity)?,
        SetComment {
            component, comment, ..
        } => m1_project::set_comment(&xml, component, comment)?,
        SetValidation {
            component,
            r#type,
            min,
            max,
            ..
        } => m1_project::set_validation(&xml, component, r#type, *min, *max)?,
        SetFormat {
            component, format, ..
        } => m1_project::set_format(&xml, component, format)?,
        SetDps { component, dps, .. } => m1_project::set_dps(&xml, component, *dps)?,
        SetDisplayRange {
            component,
            min,
            max,
            ..
        } => m1_project::set_display_range(&xml, component, *min, *max)?,
        AddTag { component, tag, .. } => m1_project::add_tag(&xml, component, tag)?,
        RemoveTag { component, tag, .. } => m1_project::remove_tag(&xml, component, tag)?,
        SetCallRate { script, rate, .. } => m1_project::set_call_rate(&xml, script, rate)?,
        ListRates { .. }
        | ListSecurity { .. }
        | Validate { .. }
        | ListComponents { .. }
        | Format { .. }
        | RenameComponent { .. } => {
            unreachable!()
        }
    };

    // A new script component needs an empty backing .m1scr created on disk, as
    // M1-Build does on insert. Only on a real write — and staged BEFORE the XML
    // write (#90): a file-creation failure must not leave a half-committed
    // project whose XML references a script that was never created. If the XML
    // write itself then fails, the staged (empty, just-created) file is removed
    // so the project directory is exactly as before.
    let staged: Option<PathBuf> = if !cli.dry_run
        && !cli.stdout
        && let CreateScheduledFunction { name, .. } | CreateFunction { name, .. } = &cli.command
    {
        create_script_file(project, name)?
    } else {
        None
    };
    match write_or_print(cli, project, &xml, &out) {
        Ok(code) => Ok(code),
        Err(e) => {
            if let Some(p) = &staged
                && let Err(re) = std::fs::remove_file(p)
            {
                eprintln!(
                    "warning: could not remove staged backing script {}: {re}",
                    p.display()
                );
            }
            Err(e)
        }
    }
}

/// Findings for script components whose backing `.m1scr` **exists but is empty** —
/// the CLI's file-aware mirror of M1-Build's "Missing code" (Error 1024).
///
/// IMPORTANT: an *absent* `.m1scr` is NOT a finding. Many components (library/base
/// method slots — `Calculation`, `Transform`, `SetState`, `Startup`, …) carry no
/// project script and inherit their behaviour; M1-Build does not flag those, and
/// neither do we (verified: the real AV-M1 project has 58 such codeless components
/// and M1-Build's Validate reports 0 errors for them). Only a present-but-empty
/// file — the stub M1-Build leaves when you insert a function and write no code —
/// is the "Missing code" error.
fn missing_code_findings(project: &Path, xml: &str) -> Vec<m1_project::Finding> {
    let Ok(scripts) = m1_project::script_components(xml) else {
        return Vec::new();
    };
    let dir = scripts_dir(project);
    let mut out = Vec::new();
    for s in scripts {
        // Only a file that EXISTS and is empty/whitespace counts; a missing file
        // means the component inherits its code and is legitimately script-less.
        // Read tolerantly (Windows-1252 fallback) — a raw UTF-8 read of a `.m1scr`
        // carrying a `°` byte would error and silently skip the check, which
        // AGENTS.md forbids for MoTeC files.
        if let Ok(body) = m1_workspace::read_text(&dir.join(&s.filename))
            && body.trim().is_empty()
        {
            out.push(m1_project::Finding {
                level: m1_project::FindingLevel::Error,
                path: s.path.clone(),
                message: format!("missing code: backing script `{}` is empty", s.filename),
                code: Some(1024),
            });
        }
    }
    out
}

fn selected_module_xmls(
    project_xml: &str,
    explicit_dirs: &[PathBuf],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let doc = roxmltree::Document::parse(project_xml)?;
    let selected: Vec<String> = doc
        .descendants()
        .find(|node| node.has_tag_name("SelectedModuleSets"))
        .into_iter()
        .flat_map(|sets| sets.children().filter(|node| node.has_tag_name("File")))
        .filter_map(|file| {
            let name = file.attribute("Name")?;
            let major = normalise_version_part(file.attribute("VersionMajor")?);
            let minor = normalise_version_part(file.attribute("VersionMinor")?);
            let build = normalise_version_part(file.attribute("VersionBuild")?);
            Some(format!("{name}.{major}.{minor}.{build}.m1mod"))
        })
        .collect();

    let search_dirs = module_search_dirs(explicit_dirs);
    let mut xmls = Vec::new();
    for filename in selected {
        let Some(path) = search_dirs
            .iter()
            .map(|dir| dir.join(&filename))
            .find(|path| path.is_file())
        else {
            continue;
        };
        let xml = m1_workspace::read_text(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        xmls.push(xml);
    }
    Ok(xmls)
}

fn normalise_version_part(part: &str) -> &str {
    let trimmed = part.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn module_search_dirs(explicit_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = if explicit_dirs.is_empty() {
        std::env::var_os("M1_MODULES_PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default()
    } else {
        explicit_dirs.to_vec()
    };
    if explicit_dirs.is_empty() {
        if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
            dirs.push(
                PathBuf::from(program_data)
                    .join("MoTeC")
                    .join("M1")
                    .join("Build")
                    .join("Modules"),
            );
        }
        // WSL can run the Linux build while sharing the host M1-Build install.
        dirs.push(PathBuf::from("/mnt/c/ProgramData/MoTeC/M1/Build/Modules"));
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// The project's `Scripts/` directory (sibling of `Project.m1prj`).
fn scripts_dir(project: &Path) -> PathBuf {
    project
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Scripts")
}

/// File-aware DBC checks (#84): for every DBCRoot module, find the backing
/// `<module>.m1dbc` within the workspace that governs the project, then check its
/// internal `BuiltIn.CAN.DBC` Name/MD5 and `<List>`/`<Organisation>` view.
///
/// This deliberately uses m1-workspace's config discovery and DBC file walk
/// instead of parsing `[dbc]` a second time. A project-local match wins. Outside
/// the project, a unique filename match wins; when several exist, the imported
/// module name and MD5 must identify exactly one. Ambiguity is an error rather
/// than a guess at another project's sources.
fn dbc_findings(project: &Path, xml: &str) -> Vec<m1_project::Finding> {
    let Ok(modules) = m1_project::dbc_modules(xml) else {
        return Vec::new();
    };
    let project_dir = project
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let project_dir = std::path::absolute(project_dir).unwrap_or_else(|_| project_dir.into());
    let search_root = m1_workspace::find_upward(&project_dir, m1_workspace::TOOLS_CONFIG_FILE)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| project_dir.clone());
    let dbc_files = m1_workspace::find_dbc_files(&search_root);
    let mut out = Vec::new();
    for m in modules {
        let mut candidates: Vec<&PathBuf> = dbc_files
            .iter()
            .filter(|path| {
                path.file_stem().and_then(|stem| stem.to_str()) == Some(m.module.as_str())
            })
            .collect();
        let local: Vec<&PathBuf> = candidates
            .iter()
            .copied()
            .filter(|path| path.starts_with(&project_dir))
            .collect();
        if !local.is_empty() {
            candidates = local;
        }
        let path = if candidates.len() == 1 {
            candidates[0]
        } else {
            let matching: Vec<&PathBuf> = candidates
                .iter()
                .copied()
                .filter(|path| dbc_file_matches_import(path, &m))
                .collect();
            if matching.len() == 1 {
                matching[0]
            } else {
                let detail = if candidates.is_empty() {
                    format!(
                        "no matching file was found under `{}`",
                        search_root.display()
                    )
                } else {
                    let names = candidates
                        .iter()
                        .map(|path| dbc_display_path(path, &search_root))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("the candidates are {names}")
                };
                out.push(m1_project::Finding {
                    level: m1_project::FindingLevel::Error,
                    path: m.name.clone(),
                    message: format!(
                        "DBC module file `{}.m1dbc` cannot be located unambiguously: {detail}",
                        m.module
                    ),
                    code: None,
                });
                continue;
            }
        };
        let display = dbc_display_path(path, &search_root);
        let Ok(body) = m1_workspace::read_text(path) else {
            out.push(m1_project::Finding {
                level: m1_project::FindingLevel::Error,
                path: m.name.clone(),
                message: format!(
                    "DBC module file `{display}` is unreadable — M1-Build cannot load the CAN database"
                ),
                code: None,
            });
            continue;
        };
        // The file is discovered by the module name, so its stem IS the module.
        match m1_project::validate_dbc_file(
            &body,
            &m.name,
            &m.module,
            m.md5.as_deref(),
            &m.module,
            &display,
        ) {
            Ok(fs) => out.extend(fs),
            Err(_) => out.push(m1_project::Finding {
                level: m1_project::FindingLevel::Error,
                path: m.name.clone(),
                message: format!("DBC module file `{display}` is not well-formed XML"),
                code: None,
            }),
        }
    }
    out
}

/// Whether a same-stem candidate identifies the database imported by `module`.
/// Used only to disambiguate; the full validation still runs after selection.
fn dbc_file_matches_import(path: &Path, module: &m1_project::DbcModule) -> bool {
    let Ok(text) = m1_workspace::read_text(path) else {
        return false;
    };
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return false;
    };
    doc.descendants().any(|node| {
        node.attribute("Classname") == Some("BuiltIn.CAN.DBC")
            && node.attribute("Name") == Some(module.module.as_str())
            && module.md5.as_deref().is_none_or(|expected| {
                node.attribute("MD5")
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
            })
    })
}

fn dbc_display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Create the empty backing `.m1scr` for a newly-created script component, as
/// M1-Build does on insert. Creates `Scripts/` if absent; never clobbers an
/// existing file. Returns the path it created (`None` when a file already
/// existed and was left as-is) so the caller can roll the creation back if the
/// XML write fails (#90).
fn create_script_file(
    project: &Path,
    name: &str,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let dir = scripts_dir(project);
    let path = dir.join(m1_project::script_relpath(name));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    if path.exists() {
        eprintln!(
            "backing script already exists, left as-is: {}",
            path.display()
        );
        Ok(None)
    } else {
        std::fs::File::create(&path).map_err(|e| format!("creating {}: {e}", path.display()))?;
        eprintln!("Created {}", path.display());
        Ok(Some(path))
    }
}

/// Validate the complete rename plan before any file is touched (#89): every
/// destination whose source will actually move must be free. `std::fs::rename`
/// replaces an existing destination on Unix, so proceeding would silently
/// destroy an unrelated (e.g. orphaned) script's bytes — unrecoverable by the
/// rollback, which can only move files back. The one allowed "occupied"
/// destination is the source itself under a different case (a case-only rename
/// on a case-insensitive filesystem). Duplicate destinations in one plan are
/// rejected for the same reason: the second rename would clobber the first.
fn preflight_renames(
    project: &Path,
    renames: &[m1_project::ScriptRename],
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = scripts_dir(project);
    let mut dests: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for r in renames {
        let from = dir.join(&r.old);
        let to = dir.join(&r.new);
        if !from.exists() {
            // The rename loop will skip this source with a warning; an occupied
            // destination for a skipped rename is not touched, so not an error.
            continue;
        }
        if !dests.insert(r.new.as_str()) {
            return Err(format!(
                "duplicate rename destination {}: refusing, the second rename would \
                 overwrite the first",
                to.display()
            )
            .into());
        }
        if to.exists() && !is_same_file(&from, &to) {
            return Err(format!(
                "rename destination already exists: {} (refusing to overwrite it; move \
                 the existing file aside first, project unchanged)",
                to.display()
            )
            .into());
        }
    }
    Ok(())
}

/// Whether two paths resolve to the same file — true for a case-only rename on
/// a case-insensitive filesystem, where the destination "exists" but IS the
/// source. Resolution failure counts as "not the same" (the preflight then
/// refuses, the safe direction).
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Rename backing `.m1scr` files to follow a `rename_component` (old → new),
/// matching M1-Build's UI. Skips any whose source file is absent. Runs BEFORE
/// the XML write (#49); on a mid-loop failure every completed rename is rolled
/// back so the project (old XML + old filenames) stays loadable. Returns the
/// renames actually performed so the caller can roll back if the XML write
/// itself fails.
fn rename_script_files(
    project: &Path,
    renames: &[m1_project::ScriptRename],
) -> Result<Vec<m1_project::ScriptRename>, Box<dyn std::error::Error>> {
    preflight_renames(project, renames)?;
    let dir = scripts_dir(project);
    let mut done: Vec<m1_project::ScriptRename> = Vec::new();
    for r in renames {
        let from = dir.join(&r.old);
        let to = dir.join(&r.new);
        if from.exists() {
            if let Err(e) = std::fs::rename(&from, &to) {
                rollback_renames(project, &done);
                return Err(format!(
                    "renaming {} -> {} failed ({e}); previous renames rolled back, project unchanged",
                    from.display(),
                    to.display()
                )
                .into());
            }
            eprintln!("Renamed {} -> {}", from.display(), to.display());
            done.push(r.clone());
        } else {
            eprintln!(
                "warning: backing script not found, skipped: {}",
                from.display()
            );
        }
    }
    Ok(done)
}

/// Undo completed `.m1scr` renames (new → old), best-effort: each failure is
/// reported but does not stop the remaining rollbacks.
fn rollback_renames(project: &Path, done: &[m1_project::ScriptRename]) {
    let dir = scripts_dir(project);
    for r in done.iter().rev() {
        let from = dir.join(&r.new);
        let to = dir.join(&r.old);
        if let Err(e) = std::fs::rename(&from, &to) {
            eprintln!(
                "warning: rollback of {} -> {} failed: {e}",
                from.display(),
                to.display()
            );
        } else {
            eprintln!("Rolled back {} -> {}", from.display(), to.display());
        }
    }
}

/// Route the edited XML: `--stdout` prints the raw result, `--dry-run` prints
/// a unified diff of what would change (#51), otherwise write back to the
/// project file.
fn write_or_print(
    cli: &Cli,
    project: &Path,
    original: &str,
    out: &str,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    if cli.stdout {
        print!("{out}");
    } else if cli.dry_run {
        let name = project.display().to_string();
        let diff = m1_workspace::diff::unified_diff(&name, original, out);
        if diff.is_empty() {
            eprintln!("dry-run: no changes to {name}");
        } else {
            print!("{diff}");
        }
    } else {
        // Defense in depth: never write XML that isn't well-formed. The surgical
        // edits are parser-located and validated by tests, but re-parsing the
        // result before the irreversible write guarantees a bug can never persist
        // corruption to the canonical project file (#5).
        if let Err(e) = roxmltree::Document::parse(out) {
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
        let encoding = motec_write_encoding(out);
        let bytes = m1_workspace::encode_checked(out, encoding)
            .map_err(|e| format!("cannot save in the file's {encoding:?} encoding: {e}"))?;
        // Atomic write: a temp file in the same directory, fsync'd, then renamed
        // over the target — an interruption/panic/ENOSPC can no longer truncate
        // the irreplaceable project file mid-write (#6). `m1_workspace::atomic_write`
        // also preserves the existing file's permission mode, so a tightened
        // `0o600` Project.m1prj is not silently widened on every edit.
        m1_workspace::atomic_write(project, &bytes)?;
        eprintln!("Updated {}", project.display());
    }
    Ok(ExitCode::SUCCESS)
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
    // Snap the 256-byte head window down to a char boundary: a naive
    // `&xml[..256]` panics when a multi-byte UTF-8 char straddles offset 256
    // (#54). `floor_char_boundary` is still unstable, so walk down to the
    // nearest boundary at or below the cap.
    let mut end = xml.len().min(256);
    while end > 0 && !xml.is_char_boundary(end) {
        end -= 1;
    }
    let head = &xml[..end];
    if let Some(end) = head.find("?>") {
        let decl = head[..end].to_ascii_lowercase();
        if decl.contains("encoding=\"utf-8\"") || decl.contains("encoding='utf-8'") {
            return m1_workspace::Encoding::Utf8;
        }
    }
    m1_workspace::Encoding::Windows1252
}

/// Produce a JSON string literal. Escapes everything RFC 8259 §7 requires:
/// quote, backslash, and all control characters U+0000–U+001F (#50) — a raw
/// newline in a component comment previously produced invalid JSON.
fn json_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Produce a JSON string literal or `null` for an absent optional.
fn json_string_or_null(s: Option<&str>) -> String {
    match s {
        Some(v) => json_string(v),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_string_escapes_everything_rfc8259_requires() {
        // #50: quote, backslash, and ALL of U+0000-U+001F.
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string("a\rb"), "\"a\\rb\"");
        assert_eq!(json_string("a\tb"), "\"a\\tb\"");
        assert_eq!(json_string("a\u{8}b"), "\"a\\bb\"");
        assert_eq!(json_string("a\u{c}b"), "\"a\\fb\"");
        assert_eq!(json_string("a\u{1}b"), "\"a\\u0001b\"");
        assert_eq!(json_string("a\u{1f}b"), "\"a\\u001fb\"");
        // Non-control unicode passes through unescaped.
        assert_eq!(json_string("°C"), "\"°C\"");
    }

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

    #[test]
    fn motec_write_encoding_no_panic_on_multibyte_char_across_byte_256() {
        // #54: the 256-byte head window must be snapped to a char boundary.
        // Build an XML doc with no `?>` in the first 256 bytes and a 2-byte
        // UTF-8 char (`°`, 0xC2 0xB0) straddling byte offset 256 — a naive
        // `&xml[..256]` slice panics on the non-boundary index.
        let mut xml = String::from("<?xml version=\"1.0\"?>");
        // Pad so that the next char's first byte lands at offset 255, leaving
        // its second byte at offset 256 (the slice boundary falls mid-char).
        while xml.len() < 255 {
            xml.push('a');
        }
        assert_eq!(xml.len(), 255);
        xml.push('°'); // bytes 255..257 — boundary 256 is inside this char
        assert!(!xml.is_char_boundary(256));
        // Must not panic; this doc has no utf-8 declaration → Windows-1252.
        assert_eq!(
            motec_write_encoding(&xml),
            m1_workspace::Encoding::Windows1252
        );
    }

    #[test]
    fn json_string_escapes_quotes() {
        assert_eq!(json_string(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn json_string_or_null_absent() {
        assert_eq!(json_string_or_null(None), "null");
        assert_eq!(json_string_or_null(Some("rpm")), "\"rpm\"");
    }
}
