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
pub fn validate(xml: &str) -> Result<Vec<Finding>, EditError> {
    let doc = parse_xml(xml)?;
    let mut findings: Vec<Finding> = Vec::new();

    // Build a set of all component names for fast lookup.
    // Only real components (those with a Classname attribute) — the <Organisation>
    // section also contains <Component> nodes without Classname that are view-only
    // structural nodes and must not participate in any validation check.
    let all_names: std::collections::HashSet<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect();

    // Build the set of valid clock absolute paths for trigger resolution.
    let valid_clocks: std::collections::HashSet<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter(|n| n.attribute("Classname") == Some("BuiltIn.EventKernel"))
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect();

    // Check 2: duplicate sibling Names.
    // Walk each Component's parent and check that no two direct children share a Name.
    // We detect "siblings" as components whose parent paths are the same.
    // Only real components (Classname present) — Organisation nodes are excluded.
    {
        // Group names by parent path.
        let mut by_parent: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for n in doc
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            if let Some(nm) = n.attribute("Name") {
                let parent_key = parent_of(nm).unwrap_or("").to_string();
                by_parent
                    .entry(parent_key)
                    .or_default()
                    .push(nm.to_string());
            }
        }
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

    // Check 3: SelectedTrigger resolution.
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
    {
        let Some(owner) = n.attribute("Name") else {
            continue;
        };
        let Some(trigger) = n
            .children()
            .find(|c| c.has_tag_name("Props"))
            .and_then(|p| p.attribute("SelectedTrigger"))
        else {
            continue;
        };
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

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.message.cmp(&b.message)));
    Ok(findings)
}

// ---- list-components --------------------------------------------------------
