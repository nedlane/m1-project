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
    /// Validate the project for structural correctness (read-only; exit 1 on findings).
    Validate {
        #[arg(long)]
        project: PathBuf,
        /// Emit JSON (array of objects with level/path/message) instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List all components in the project.
    ListComponents {
        #[arg(long)]
        project: PathBuf,
        /// Emit JSON (array of objects with path/classname/type/unit/security/call_rate).
        #[arg(long)]
        json: bool,
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
            | Command::Validate { project, .. }
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
        Validate { project, json } => {
            let (xml, _enc) = m1_workspace::read_text_with_encoding(project)
                .map_err(|e| format!("{}: {e}", project.display()))?;
            let mut findings = m1_project::validate(&xml)?;
            // File-aware check (only the CLI does I/O): a script component whose
            // backing `.m1scr` is missing or empty is M1-Build's "Missing code"
            // error. `validate()` itself stays pure (`&str` → findings).
            findings.extend(missing_code_findings(project, &xml));
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
                    println!(
                        "  {{\"level\":{},\"path\":{},\"message\":{}}}{}",
                        json_string(level),
                        json_string(&f.path),
                        json_string(&f.message),
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
                        "  {{\"path\":{},\"classname\":{},\"type\":{},\"unit\":{},\"security\":{},\"call_rate\":{},\"qty\":{},\"tags\":{},\"comment\":{}}}{}",
                        json_string(&e.path),
                        json_string(&e.classname),
                        ty_json,
                        unit_json,
                        sec_json,
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
        let code = write_or_print(cli, project, &xml, &out)?;
        // On a real write, rename the backing .m1scr files to follow the component
        // (M1-Build does this in its UI). --dry-run/--stdout leave the disk alone.
        if !cli.dry_run && !cli.stdout {
            rename_script_files(project, &script_renames)?;
        }
        return Ok(code);
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
        ListRates { .. } | Validate { .. } | ListComponents { .. } | RenameComponent { .. } => {
            unreachable!()
        }
    };

    let code = write_or_print(cli, project, &xml, &out)?;
    // A new script component needs an empty backing .m1scr created on disk, as
    // M1-Build does on insert. Only on a real write.
    if !cli.dry_run
        && !cli.stdout
        && let CreateScheduledFunction { name, .. } | CreateFunction { name, .. } = &cli.command
    {
        create_script_file(project, name)?;
    }
    Ok(code)
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
        if let Ok(body) = std::fs::read_to_string(dir.join(&s.filename))
            && body.trim().is_empty()
        {
            out.push(m1_project::Finding {
                level: m1_project::FindingLevel::Error,
                path: s.path.clone(),
                message: format!("missing code: backing script `{}` is empty", s.filename),
            });
        }
    }
    out
}

/// The project's `Scripts/` directory (sibling of `Project.m1prj`).
fn scripts_dir(project: &Path) -> PathBuf {
    project
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Scripts")
}

/// Create the empty backing `.m1scr` for a newly-created script component, as
/// M1-Build does on insert. Creates `Scripts/` if absent; never clobbers an
/// existing file.
fn create_script_file(project: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = scripts_dir(project);
    let path = dir.join(m1_project::script_relpath(name));
    std::fs::create_dir_all(&dir)?;
    if path.exists() {
        eprintln!(
            "backing script already exists, left as-is: {}",
            path.display()
        );
    } else {
        std::fs::File::create(&path)?;
        eprintln!("Created {}", path.display());
    }
    Ok(())
}

/// Rename backing `.m1scr` files to follow a `rename_component` (old → new),
/// matching M1-Build's UI. Skips any whose source file is absent.
fn rename_script_files(
    project: &Path,
    renames: &[m1_project::ScriptRename],
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = scripts_dir(project);
    for r in renames {
        let from = dir.join(&r.old);
        let to = dir.join(&r.new);
        if from.exists() {
            std::fs::rename(&from, &to)?;
            eprintln!("Renamed {} -> {}", from.display(), to.display());
        } else {
            eprintln!(
                "warning: backing script not found, skipped: {}",
                from.display()
            );
        }
    }
    Ok(())
}

/// Either print to stdout (dry-run / --stdout) or write back to the project file.
fn write_or_print(
    cli: &Cli,
    project: &Path,
    _original: &str,
    out: &str,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    if cli.dry_run || cli.stdout {
        print!("{out}");
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
    let head = &xml[..xml.len().min(256)];
    if let Some(end) = head.find("?>") {
        let decl = head[..end].to_ascii_lowercase();
        if decl.contains("encoding=\"utf-8\"") || decl.contains("encoding='utf-8'") {
            return m1_workspace::Encoding::Utf8;
        }
    }
    m1_workspace::Encoding::Windows1252
}

/// Produce a JSON string literal (with double-quote escaping).
fn json_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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
    fn json_string_escapes_quotes() {
        assert_eq!(json_string(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn json_string_or_null_absent() {
        assert_eq!(json_string_or_null(None), "null");
        assert_eq!(json_string_or_null(Some("rpm")), "\"rpm\"");
    }
}
