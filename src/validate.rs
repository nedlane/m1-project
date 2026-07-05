//! Read-only structural validation of a `Project.m1prj` (`validate`), and the
//! `Finding`/`FindingLevel` report types it returns.

use crate::EditError;
use crate::query::resolve_trigger;
use crate::xml::*;
use std::fmt;

/// A single validation finding.
#[derive(Debug)]
pub struct Finding {
    pub level: FindingLevel,
    pub path: String,
    pub message: String,
    /// The M1-Build error number this finding mirrors, when one is known
    /// (e.g. `1601` "No security group selected", `1338` "Object does not
    /// exist", `1024` "Missing code"). `None` for checks with no documented
    /// M1-Build code (duplicate-name, blank-name, missing-trigger). Surfaced in
    /// `validate --json` as a machine-readable `code` field so a CI consumer can
    /// triage by error number without parsing the free-text `message`.
    pub code: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingLevel {
    Error,
    Warning,
}

impl fmt::Display for FindingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FindingLevel::Error => write!(f, "ERROR"),
            FindingLevel::Warning => write!(f, "WARN"),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.level, self.path, self.message)
    }
}

/// Validate a project XML for structural correctness.  Returns a list of all
/// findings (not fail-fast); the caller decides on exit code (non-empty → fail).
///
/// Checks performed:
/// 1. XML parses without error (the file is well-formed and decodable).
/// 2. No two siblings share the same `Name` attribute value.
/// 3. Every `SelectedTrigger` resolves either to `"startup"` or to an existing
///    `BuiltIn.EventKernel` component under `Root.Events`.
/// 4. The `<List>` and `<Organisation>` view tree agree (a view node with no real
///    component is an error — M1-Build fails to load; a component missing from the
///    view is a warning).
/// 5. Every scheduled function (`BuiltIn.FuncUser`) has an event/trigger selected
///    (mirrors M1-Build's "no event selected" — such a function never runs).
/// 6. Every value component (`BuiltIn.Channel`/`BuiltIn.Parameter`) has a
///    `<Props Security>` (mirrors M1-Build Error 1601 "No security group selected").
/// 7. Every component's `<Props Security>` value is one of the project's declared
///    security groups (`<SecurityMgr><SecurityRoles>`). M1-Build will not bind a
///    component to a group the project does not declare. Skipped entirely for
///    projects with no `<SecurityMgr>` (Automatic / tag-derived security mode),
///    where there is no explicit role list to check against.
/// 8. No component's `Name` last segment is empty or whitespace-only — such a
///    blank name is not a usable named object in M1-Build (defence-in-depth for
///    files written by an older build or hand-edited; the create/rename verbs
///    already refuse to produce one).
/// 9. Every component reference (table Axis `Source`, `Reference` `Target`,
///    `NameTarget`) resolves to a real component (M1-Build Error 1338).
/// 10. No DBCRoot module (`BuiltIn.CAN.DBC`) carries the all-zero MD5 sentinel,
///     and no two modules share an MD5 — M1-Build refuses to open the project in
///     either case (#82).
/// 11. No DBCRoot module name (`DBC.<name>`) collides with a `Root.CAN.<name>` /
///     `Root.Control.<name>` project object — a Warning, since the exact scope of
///     the clash is not fully known (#83).
pub fn validate(xml: &str) -> Result<Vec<Finding>, EditError> {
    let doc = parse_xml(xml)?;
    let mut findings: Vec<Finding> = Vec::new();

    // The project's declared security groups, if it declares any (Check 7).
    // `None` => no <SecurityMgr> (Automatic security mode) => skip the check.
    let declared_roles: Option<std::collections::HashSet<String>> =
        declared_security_roles(xml)?.map(|roles| roles.into_iter().collect());

    // ONE pass over the document fills every accumulator the checks below need;
    // validate() used to make eight separate `descendants()` traversals, and it
    // wraps every mutating verb, so large projects paid 8× the necessary
    // tree-walk cost per edit (#40). Only real components (those with a
    // Classname attribute) participate — the <Organisation> section also
    // contains <Component> nodes without Classname that are view-only
    // structural nodes; they are collected separately for check 4.
    let mut all_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut valid_clocks: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut by_parent: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    // (owner, trigger) pairs for check 3 — resolution needs `valid_clocks`
    // complete, so it runs after the pass.
    let mut triggered: Vec<(String, String)> = Vec::new();
    // (owner, attr, value) for the reference-resolution check (Check 9).
    let mut references: Vec<(String, &'static str, String)> = Vec::new();
    let mut org_roots: Vec<roxmltree::Node> = Vec::new();
    // DBCRoot module entries (`<Component Classname="BuiltIn.CAN.DBC" MD5=…
    // Name="DBC.<module>">`) for the DBC hash checks (Check 10) and the
    // module/object name-collision check (Check 11). The `MD5` is an attribute on
    // the `<Component>` element itself, not on `<Props>`. The `BuiltIn.CAN.DBCRoot`
    // container (no MD5) is deliberately excluded.
    let mut dbc_modules: Vec<(String, Option<String>)> = Vec::new();

    for n in doc.descendants() {
        if n.has_tag_name("Organisation") {
            org_roots.push(n);
            continue;
        }
        // Only real components (carrying a Classname) participate — the
        // <Organisation> view nodes are excluded by the same single-source
        // predicate every other pass uses.
        if !is_real_component(&n) {
            continue;
        }
        let classname = n
            .attribute("Classname")
            .expect("is_real_component checked Classname");
        let Some(nm) = n.attribute("Name") else {
            continue;
        };
        let props = n.children().find(|c| c.has_tag_name("Props"));
        let trigger = props.and_then(|p| p.attribute("SelectedTrigger"));

        all_names.insert(nm.to_string());
        if classname == "BuiltIn.EventKernel" {
            valid_clocks.insert(nm.to_string());
        }
        // A `BuiltIn.CAN.DBC` component (NOT the `BuiltIn.CAN.DBCRoot` container)
        // names an imported CAN database as `DBC.<module>` and carries the
        // imported file's `MD5` on the element itself (Checks 10/11).
        if classname == "BuiltIn.CAN.DBC" {
            dbc_modules.push((nm.to_string(), n.attribute("MD5").map(str::to_string)));
        }

        // Check 8 (defence-in-depth for already-corrupt files): a component whose
        // Name's last segment is whitespace-only (e.g. `Name="Root.  "`) is not a
        // usable named object in M1-Build. The create/rename verbs refuse to
        // produce one (see `validate_name_segment`), but a file written by an
        // older build or hand-edited can still carry one.
        let seg = nm.rsplit('.').next().unwrap_or(nm);
        if seg.trim().is_empty() {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: nm.to_string(),
                message: "component Name segment is empty or whitespace-only — M1-Build cannot use it as a named object"
                    .into(),
                code: None,
            });
        }
        by_parent
            .entry(parent_of(nm).unwrap_or("").to_string())
            .or_default()
            .push(nm.to_string());
        if let Some(t) = trigger {
            triggered.push((nm.to_string(), t.to_string()));
        }
        // Component references (table Axis Source, Reference Target, NameTarget)
        // for Check 9.
        if let Some(p) = props {
            for attr in crate::query::REFERENCE_ATTRS {
                if let Some(v) = p.attribute(attr) {
                    references.push((nm.to_string(), attr, v.to_string()));
                }
            }
        }

        // Check 5: a scheduled function (BuiltIn.FuncUser) with no event/trigger.
        // M1-Build reports this as an error ("no event selected") in Validate
        // Project — the function would never be scheduled, so it never runs.
        // (FuncUserParam functions are *called* by other code, not scheduled, so
        // they legitimately have no trigger and are excluded.) A `$(…)`
        // expression trigger counts as selected.
        if classname == "BuiltIn.FuncUser" && trigger.map(|t| t.trim().is_empty()).unwrap_or(true) {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: nm.to_string(),
                message:
                    "scheduled function has no event selected (SelectedTrigger) — it will never run"
                        .into(),
                code: None,
            });
        }

        // Check 6: a value component (Channel/Parameter) with no security group.
        // M1-Build requires every channel/parameter to have a Security level and
        // reports "No security group selected" (Error 1601) otherwise. Verified
        // safe: all 737 channels/parameters in the real AV-M1 project carry a
        // `Security` and M1-Build reports 0 errors; a freshly-inserted bare one
        // is flagged (exactly what `create-channel`/`create-parameter` produce
        // until `set-security`).
        if matches!(classname, "BuiltIn.Channel" | "BuiltIn.Parameter")
            && props.and_then(|p| p.attribute("Security")).is_none()
        {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: nm.to_string(),
                message: "no security group selected — a channel/parameter needs a Security level"
                    .into(),
                code: Some(1601),
            });
        }

        // Check 7: a Security value that is not one of the project's declared
        // groups. Security groups are project-defined (<SecurityMgr>
        // <SecurityRoles>); M1-Build will not bind a component to a group the
        // project does not declare, so an undeclared value is an error. Only
        // checked when the project declares roles explicitly — Automatic-mode
        // projects (no <SecurityMgr>) have no role list to validate against.
        if let (Some(roles), Some(sec)) =
            (&declared_roles, props.and_then(|p| p.attribute("Security")))
            && !roles.contains(sec)
        {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: nm.to_string(),
                message: format!(
                    "Security group `{sec}` is not declared in the project's <SecurityRoles> — M1-Build cannot bind it"
                ),
                code: Some(1601),
            });
        }
    }

    // Check 2: duplicate sibling Names — no two direct children of one parent
    // path may share a Name segment.
    {
        for (parent_key, siblings) in &by_parent {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            let mut duped: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for nm in siblings {
                // The sibling name segment is the last dot-segment.
                let seg = nm.rsplit('.').next().unwrap_or(nm.as_str());
                if !seen.insert(seg) {
                    duped.insert(seg);
                }
            }
            for seg in duped {
                let path = if parent_key.is_empty() {
                    seg.to_string()
                } else {
                    format!("{parent_key}.{seg}")
                };
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: path.clone(),
                    message: format!("duplicate sibling Name `{seg}` under `{parent_key}`"),
                    code: None,
                });
            }
        }
    }

    // Check 3: SelectedTrigger resolution (over the pairs collected above —
    // resolution needs the complete clock set).
    for (owner, trigger) in &triggered {
        let (owner, trigger) = (owner.as_str(), trigger.as_str());
        // "On Startup" is always valid (no clock component needed in some projects).
        if trigger.eq_ignore_ascii_case("startup")
            || trigger.ends_with(".On Startup")
            || trigger == "On Startup"
        {
            continue;
        }
        // M1 Build expression references — `$(Path:Attribute)` — inherit the value
        // of a named attribute from another component at runtime.  The string is not
        // a literal path and cannot be statically resolved; skip validation.
        if trigger.starts_with("$(") {
            continue;
        }
        match resolve_trigger(owner, trigger) {
            None => {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: owner.to_string(),
                    message: format!(
                        "cannot resolve SelectedTrigger `{trigger}` (malformed relative path)"
                    ),
                    code: None,
                });
            }
            Some(abs) => {
                if !all_names.contains(&abs) || !valid_clocks.contains(&abs) {
                    findings.push(Finding {
                        level: FindingLevel::Error,
                        path: owner.to_string(),
                        message: format!(
                            "SelectedTrigger `{trigger}` resolves to `{abs}` which is not a BuiltIn.EventKernel clock"
                        ),
                        code: None,
                    });
                }
            }
        }
    }

    // Check 9: component references resolve. A table Axis `Source`, a
    // `BuiltIn.Reference` `Target`, or a `NameTarget` that points at a component
    // (or its implicit `.Value`/`.Resource` child) which does not exist is a
    // dangling reference — M1-Build's "Object does not exist" (error 1338).
    // rename/delete used to leave these dangling and validate never caught it,
    // so the toolchain could both create and certify a broken project. Verified
    // false-positive-free on the real corpora (every EV-M1/AV-M1 reference
    // resolves). A `$(…)` template is dynamic and skipped.
    for (owner, attr, value) in &references {
        if value.starts_with("$(") {
            continue;
        }
        match crate::query::resolve_reference(owner, value) {
            None => findings.push(Finding {
                level: FindingLevel::Error,
                path: owner.clone(),
                message: format!("cannot resolve {attr} `{value}` (malformed relative path)"),
                code: None,
            }),
            Some(abs) => {
                if !crate::query::reference_resolves(&abs, |p| all_names.contains(p)) {
                    findings.push(Finding {
                        level: FindingLevel::Error,
                        path: owner.clone(),
                        message: format!(
                            "{attr} `{value}` resolves to `{abs}`, which is not a component \
                             (dangling reference — M1-Build Error 1338)"
                        ),
                        code: None,
                    });
                }
            }
        }
    }

    // Check 4: <List> / <Organisation> consistency. M1-Build binds each object's
    // Properties through the <Organisation> view tree, so the two must agree:
    //   - a view node with no matching real component makes M1-Build FAIL TO LOAD
    //     the project ("Unable to find Properties for object 'Root.X'"), and
    //   - a real component absent from the view tree cannot be bound either:
    //     M1-Build never builds its Properties, so scripts referencing it error
    //     1338 ("Object/Local/Method does not exist"). Both halves are fatal.
    // (Projects without any <Organisation> skip this check entirely.)
    if !org_roots.is_empty() {
        let mut org_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
        for org in &org_roots {
            collect_org_paths(*org, "", &mut org_paths);
        }
        for p in &org_paths {
            if !all_names.contains(p) {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: p.clone(),
                    message:
                        "<Organisation> view references a component missing from <List> (M1-Build cannot bind its Properties)"
                            .into(),
                    code: Some(1338),
                });
            }
        }
        for nm in &all_names {
            if !org_paths.contains(nm) {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: nm.clone(),
                    message: "component is absent from the <Organisation> view (M1-Build cannot bind its Properties; references error 1338)"
                        .into(),
                    code: Some(1338),
                });
            }
        }
    }

    // Check 10: DBC module hashes (#82). M1-Build refuses to OPEN a project whose
    // DBCRoot carries the all-zero MD5 sentinel (a module that was never imported)
    // or the SAME MD5 on two different `BuiltIn.CAN.DBC` modules (a duplicate
    // import). Both are hard load failures, so both are Errors. Verified
    // false-positive-free on the real corpora — every DBC module there has a
    // distinct, non-zero MD5.
    {
        const ZERO_MD5: &str = "00000000000000000000000000000000";
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (name, md5) in &dbc_modules {
            let Some(md5) = md5 else { continue };
            if md5.eq_ignore_ascii_case(ZERO_MD5) {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: name.clone(),
                    message:
                        "DBC module has the all-zero MD5 sentinel — the CAN database was never imported; M1-Build cannot open the project"
                            .into(),
                    code: None,
                });
                continue;
            }
            if let Some(prev) = seen.insert(md5.to_ascii_lowercase(), name.clone()) {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: name.clone(),
                    message: format!(
                        "duplicate DBC module hash: MD5 `{md5}` is already used by `{prev}` — M1-Build cannot open a project with two identical CAN-database imports"
                    ),
                    code: None,
                });
            }
        }
    }

    // Check 11: a DBC module name that collides with a project object (#83).
    // M1-Build has been observed to fail to open a project when a DBCRoot module
    // `DBC.<name>` shares its leaf `<name>` with a direct child of `Root.CAN` or
    // `Root.Control` (e.g. `DBC.PDM` vs `Root.CAN.PDM`). The exact scope is not
    // fully known — a DEEPER object such as `Root.Control.AV.DFMM` coexists with
    // `DBC.DFMM` and loads fine — so this is a WARNING, gated to the two clearest
    // collision sites to stay false-positive-free on the real corpora (which load
    // in M1-Build today).
    for (name, _) in &dbc_modules {
        let Some(leaf) = name.strip_prefix("DBC.") else {
            continue;
        };
        for parent in ["Root.CAN", "Root.Control"] {
            let candidate = format!("{parent}.{leaf}");
            if all_names.contains(&candidate) {
                findings.push(Finding {
                    level: FindingLevel::Warning,
                    path: name.clone(),
                    message: format!(
                        "DBC module name `{leaf}` collides with the project object `{candidate}` — M1-Build may fail to open the project (a CAN-database name and an object name must not clash)"
                    ),
                    code: None,
                });
            }
        }
    }

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.message.cmp(&b.message)));
    Ok(findings)
}

/// A DBCRoot module entry: the `BuiltIn.CAN.DBC` component `Name` (`DBC.<module>`),
/// its `<module>` leaf (the `dbc/<module>.m1dbc` file stem it maps to), and the
/// imported CAN database `MD5` (an attribute on the `<Component>` element). The
/// `BuiltIn.CAN.DBCRoot` container itself is excluded.
#[derive(Debug, Clone)]
pub struct DbcModule {
    /// Fully-qualified component name, e.g. `DBC.M150`.
    pub name: String,
    /// The module leaf, e.g. `M150`.
    pub module: String,
    /// The imported CAN database MD5, if the element carries one.
    pub md5: Option<String>,
}

/// Enumerate the project's DBCRoot module entries (the `BuiltIn.CAN.DBC`
/// components). Each maps to a `dbc/<module>.m1dbc` file; the CLI's file-aware DBC
/// check ([`validate_dbc_file`]) reads and cross-checks those (#84).
pub fn dbc_modules(xml: &str) -> Result<Vec<DbcModule>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out = Vec::new();
    for n in doc.descendants().filter(is_real_component) {
        if n.attribute("Classname") != Some("BuiltIn.CAN.DBC") {
            continue;
        }
        let Some(name) = n.attribute("Name") else {
            continue;
        };
        let module = name.strip_prefix("DBC.").unwrap_or(name).to_string();
        out.push(DbcModule {
            name: name.to_string(),
            module,
            md5: n.attribute("MD5").map(str::to_string),
        });
    }
    Ok(out)
}

/// File-aware internal-consistency checks for one `.m1dbc` CAN-database file (#84).
///
/// Pure (`&str` in, findings out) so it is unit-testable; the CLI does the I/O and
/// supplies `dbc_component` (the DBCRoot entry `Name`, used as the finding path),
/// the `module` leaf and `expected_md5` from the DBCRoot entry, and the actual
/// filename `stem`. Checks:
///   - the file has an internal `BuiltIn.CAN.DBC` component;
///   - its `Name` matches the DBCRoot module and the filename stem;
///   - its `MD5` matches the DBCRoot entry (an out-of-sync re-import otherwise);
///   - the file's own `<List>`/`<Organisation>` views agree (the real AV-M1
///     `Dash.DashVals1` bug: an org node with no matching List component makes
///     M1-Build fail with "Unable to find Properties for object").
pub fn validate_dbc_file(
    dbc_xml: &str,
    dbc_component: &str,
    module: &str,
    expected_md5: Option<&str>,
    stem: &str,
) -> Result<Vec<Finding>, EditError> {
    let doc = parse_xml(dbc_xml)?;
    let mut findings = Vec::new();

    let inner = doc
        .descendants()
        .find(|n| is_real_component(n) && n.attribute("Classname") == Some("BuiltIn.CAN.DBC"));
    match inner {
        None => findings.push(Finding {
            level: FindingLevel::Error,
            path: dbc_component.to_string(),
            message: format!(
                "`{stem}.m1dbc` has no internal BuiltIn.CAN.DBC component — it is not a valid CAN database file"
            ),
            code: None,
        }),
        Some(inner) => {
            let inner_name = inner.attribute("Name").unwrap_or("");
            if inner_name != module {
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: dbc_component.to_string(),
                    message: format!(
                        "`{stem}.m1dbc` internal module name `{inner_name}` does not match the DBCRoot module `{module}`"
                    ),
                    code: None,
                });
            } else if inner_name != stem {
                // Only when it matched the DBCRoot module but not the filename
                // (the two are normally the same string) — a renamed file.
                findings.push(Finding {
                    level: FindingLevel::Error,
                    path: dbc_component.to_string(),
                    message: format!(
                        "`{stem}.m1dbc` internal module name `{inner_name}` does not match its filename stem `{stem}`"
                    ),
                    code: None,
                });
            }
            if let Some(expected) = expected_md5 {
                let inner_md5 = inner.attribute("MD5").unwrap_or("");
                if !inner_md5.eq_ignore_ascii_case(expected) {
                    findings.push(Finding {
                        level: FindingLevel::Error,
                        path: dbc_component.to_string(),
                        message: format!(
                            "`{stem}.m1dbc` internal MD5 `{inner_md5}` does not match the DBCRoot MD5 `{expected}` — the imported database is out of sync"
                        ),
                        code: None,
                    });
                }
            }
        }
    }

    findings.extend(dbc_org_list_consistency(&doc, dbc_component, stem));
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.message.cmp(&b.message)));
    Ok(findings)
}

/// `<List>`/`<Organisation>` consistency within a single `.m1dbc` document — the
/// same binding rule as the project's Check 4, applied to the CAN-database file.
/// Both halves are fatal to M1-Build (it cannot bind an object's Properties).
fn dbc_org_list_consistency(
    doc: &roxmltree::Document<'_>,
    path_prefix: &str,
    stem: &str,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let all_names: std::collections::HashSet<String> = doc
        .descendants()
        .filter(is_real_component)
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect();
    let mut org_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    for org in doc.descendants().filter(|n| n.has_tag_name("Organisation")) {
        collect_org_paths(org, "", &mut org_paths);
    }
    if org_paths.is_empty() {
        return findings;
    }
    for p in &org_paths {
        if !all_names.contains(p) {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: path_prefix.to_string(),
                message: format!(
                    "`{stem}.m1dbc`: <Organisation> node `{p}` has no matching <List> component — M1-Build cannot find its Properties"
                ),
                code: Some(1338),
            });
        }
    }
    for nm in &all_names {
        if !org_paths.contains(nm) {
            findings.push(Finding {
                level: FindingLevel::Error,
                path: path_prefix.to_string(),
                message: format!(
                    "`{stem}.m1dbc`: <List> component `{nm}` is absent from the <Organisation> view — M1-Build cannot bind its Properties"
                ),
                code: Some(1338),
            });
        }
    }
    findings
}

/// Opt-in heuristic for M1-Build's mandatory-tag warning 1142 ("a mandatory Type
/// tag group is not selected"). OFF by default — enabled only via
/// `validate --check-mandatory-tags` — because the real projects carry hundreds of
/// legitimately-untagged objects and the full tag-GROUP model (which groups are
/// mandatory, which tags belong to them) is NOT encoded in the `.m1prj`, so it
/// cannot be derived offline. The clearest safe sub-case is flagged: a
/// value-bearing Channel/Parameter/Table that carries NO user tag at all (so no
/// tag from any group, mandatory or not, is selected). Emitted as WARNINGs, which
/// never change the exit code / fail CI.
pub fn mandatory_tag_findings(xml: &str) -> Result<Vec<Finding>, EditError> {
    let doc = parse_xml(xml)?;
    let mut findings = Vec::new();
    for n in doc.descendants().filter(is_real_component) {
        let classname = n.attribute("Classname").unwrap_or("");
        if !matches!(
            classname,
            "BuiltIn.Channel" | "BuiltIn.Parameter" | "BuiltIn.Table" | "BuiltIn.CalibrationTable"
        ) {
            continue;
        }
        let Some(nm) = n.attribute("Name") else {
            continue;
        };
        let has_tag = n
            .children()
            .find(|c| c.has_tag_name("Props"))
            .and_then(|p| p.children().find(|c| c.has_tag_name("List.UserTags")))
            .map(|t| t.children().any(|e| e.has_tag_name("Entry")))
            .unwrap_or(false);
        if !has_tag {
            findings.push(Finding {
                level: FindingLevel::Warning,
                path: nm.to_string(),
                message:
                    "no user tag selected — M1-Build warns when a mandatory tag group (e.g. Type) has no tag (Build 1142)"
                        .into(),
                code: Some(1142),
            });
        }
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.message.cmp(&b.message)));
    Ok(findings)
}

/// Recursively collect the full dotted paths of every `<Organisation>` view node,
/// joining the short `Name` segments level by level (`Root` -> `Root.CAN` -> …).
fn collect_org_paths(
    node: roxmltree::Node,
    prefix: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for child in node.children().filter(|c| c.has_tag_name("Component")) {
        let Some(name) = child.attribute("Name") else {
            continue;
        };
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        collect_org_paths(child, &path, out);
        out.insert(path);
    }
}
