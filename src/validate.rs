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
    let mut org_roots: Vec<roxmltree::Node> = Vec::new();

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
            });
        }
        by_parent
            .entry(parent_of(nm).unwrap_or("").to_string())
            .or_default()
            .push(nm.to_string());
        if let Some(t) = trigger {
            triggered.push((nm.to_string(), t.to_string()));
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
                });
            }
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
