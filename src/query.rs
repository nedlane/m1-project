//! Read-only queries: enumerate components (`list_components`), the call-rate
//! catalogue (`available_rates`), the security-group catalogue
//! (`security_groups`) and group-relative trigger resolution (`resolve_trigger`).

use crate::xml::*;
use crate::{EditError, SECURITY_LEVELS};

/// A component entry returned by [`list_components`].
#[derive(Debug)]
pub struct ComponentEntry {
    /// Fully-qualified dotted name, e.g. `Root.Engine.Speed`.
    pub path: String,
    /// Classname, e.g. `BuiltIn.Channel`.
    pub classname: String,
    /// Storage type (`<Props Type>`), if present.
    pub ty: Option<String>,
    /// Display unit (`<Props><Locale><Default Unit>`), if present.
    pub unit: Option<String>,
    /// Security level (`<Props Security>`), if present.
    pub security: Option<String>,
    /// Call rate trigger (`<Props SelectedTrigger>`), if present.
    pub call_rate: Option<String>,
    /// Physical quantity (`<Props Qty>`), if present — `set-quantity`'s read-back (#43).
    pub qty: Option<String>,
    /// User tags (`<Props><List.UserTags><Entry Value>`), in document order —
    /// `add-tag`'s read-back (#43).
    pub tags: Vec<String>,
    /// Component comment (the `<Comment>` CDATA text), if non-empty (#43).
    pub comment: Option<String>,
    /// Depth in the path hierarchy (number of dots in `path`).
    pub depth: usize,
}

/// Enumerate all `<Component>` elements in document order.
///
/// Only real components (those with a `Classname` attribute) are returned — the
/// shared `is_real_component` predicate is the single source of truth for that
/// distinction. The `<Organisation>` section of a real `.m1prj` also contains
/// `<Component>` nodes without `Classname`; those are view-only structural nodes
/// and are excluded.
pub fn list_components(xml: &str) -> Result<Vec<ComponentEntry>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out = Vec::new();
    for n in doc.descendants().filter(is_real_component) {
        let Some(path) = n.attribute("Name") else {
            continue;
        };
        let classname = n.attribute("Classname").unwrap_or("").to_string();
        let props = n.children().find(|c| c.has_tag_name("Props"));
        let ty = props.and_then(|p| p.attribute("Type")).map(str::to_string);
        let security = props
            .and_then(|p| p.attribute("Security"))
            .map(str::to_string);
        let call_rate = props
            .and_then(|p| p.attribute("SelectedTrigger"))
            .map(str::to_string);
        let unit = props
            .and_then(|p| {
                p.descendants()
                    .find(|d| d.has_tag_name("Default") && d.has_attribute("Unit"))
            })
            .and_then(|d| d.attribute("Unit"))
            .map(str::to_string);
        let qty = props.and_then(|p| p.attribute("Qty")).map(str::to_string);
        let tags: Vec<String> = props
            .and_then(|p| p.children().find(|c| c.has_tag_name("List.UserTags")))
            .map(|t| {
                t.children()
                    .filter(|e| e.has_tag_name("Entry"))
                    .filter_map(|e| e.attribute("Value"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        // The comment is stored as CDATA on its own line (M1-Build's layout:
        // `<Comment>\n<![CDATA[…]]>\n  </Comment>`), so concatenate all text
        // children and trim the layout newlines.
        let comment = n
            .children()
            .find(|c| c.has_tag_name("Comment"))
            .map(|c| {
                c.children()
                    .filter_map(|t| t.text())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty());
        let depth = path.matches('.').count();
        out.push(ComponentEntry {
            path: path.to_string(),
            classname,
            ty,
            unit,
            security,
            call_rate,
            qty,
            tags,
            comment,
            depth,
        });
    }
    Ok(out)
}

/// A script component (`FuncUser`/`FuncUserParam`/legacy `MethodUser`) and the
/// backing-script path it points at — used by the CLI to check that each script
/// actually has code (M1-Build's "Missing code" error).
#[derive(Debug)]
pub struct ScriptComponent {
    /// Fully-qualified dotted name.
    pub path: String,
    /// The `Filename` relative to the project's `Scripts/` directory. Falls back
    /// to the conventional [`crate::script_relpath`] when the attribute is absent.
    pub filename: String,
}

/// Every script component in the project, with its backing `.m1scr` path. These
/// are the components whose body M1-Build compiles; an empty body is its
/// "Missing code" error.
pub fn script_components(xml: &str) -> Result<Vec<ScriptComponent>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out = Vec::new();
    for n in doc.descendants().filter(is_real_component) {
        let Some(path) = n.attribute("Name") else {
            continue;
        };
        let class = n.attribute("Classname").unwrap_or("");
        if !(class.contains("FuncUser") || class.contains("MethodUser")) {
            continue;
        }
        let filename = n
            .attribute("Filename")
            .map(str::to_string)
            .unwrap_or_else(|| crate::script_relpath(path));
        out.push(ScriptComponent {
            path: path.to_string(),
            filename,
        });
    }
    Ok(out)
}

// ---- path helpers -----------------------------------------------------------

/// Resolve a group-relative `SelectedTrigger` value from `owner` to an absolute
/// component path.  Returns `None` if the path is structurally invalid.
///
/// The trigger format is `Parent.×N.Events.On <…>`: one `Parent.` per dot in the
/// owner's path that the script wants to climb, then the rest of the absolute
/// path from that ancestor.  Example: owner `Root.Engine.Update` (2 dots →
/// climb 2) + `Parent.Parent.Events.On 100Hz` → `Root.Events.On 100Hz`.
pub fn resolve_trigger(owner: &str, trigger: &str) -> Option<String> {
    let segments: Vec<&str> = owner.split('.').collect();
    let mut climb = 0usize;
    let mut rest = trigger;
    while let Some(tail) = rest.strip_prefix("Parent.") {
        climb += 1;
        rest = tail;
    }
    if climb > segments.len() {
        return None;
    }
    let ancestor = &segments[..segments.len() - climb];
    if ancestor.is_empty() {
        // rest IS the absolute path from the root level.
        Some(rest.to_string())
    } else {
        Some(format!("{}.{rest}", ancestor.join(".")))
    }
}

/// The `<Props>` attributes that carry a component *reference* — a table axis
/// `Source`, a `BuiltIn.Reference` `Target`, and a `NameTarget`. Each is a
/// `This.`/`Parent.`-anchored (or absolute) path to another component.
pub const REFERENCE_ATTRS: [&str; 3] = ["Source", "Target", "NameTarget"];

/// Resolve a `This.`/`Parent.`-anchored (or absolute) reference to its absolute
/// path, given the referring component's own `Name`. `This` denotes the
/// referrer's enclosing group (manual p.41), each `Parent` climbs one further
/// group; an already-absolute `Root.…` value is returned as-is. Mirrors the
/// authoritative resolver in m1-typecheck.
///
/// The resolved path may end in an *implicit accessor* child (`.Value`,
/// `.Resource`, …) that is not itself a listed component; callers therefore
/// treat a reference as resolving when the path OR any of its prefixes is a real
/// component (see [`reference_resolves`]).
pub fn resolve_reference(referrer: &str, value: &str) -> Option<String> {
    let group = referrer
        .rsplit_once('.')
        .map(|(g, _)| g)
        .unwrap_or(referrer);
    if value == "This" {
        return Some(group.to_string());
    }
    if let Some(rest) = value.strip_prefix("This.") {
        return Some(format!("{group}.{rest}"));
    }
    if value.starts_with("Parent") {
        let mut rest = value;
        let mut levels = 0usize;
        loop {
            if rest == "Parent" {
                levels += 1;
                rest = "";
                break;
            }
            match rest.strip_prefix("Parent.") {
                Some(r) => {
                    rest = r;
                    levels += 1;
                }
                None => break,
            }
        }
        let mut base = group.to_string();
        for _ in 0..levels {
            let i = base.rfind('.')?;
            base.truncate(i);
        }
        return Some(if rest.is_empty() {
            base
        } else if base.is_empty() {
            rest.to_string()
        } else {
            format!("{base}.{rest}")
        });
    }
    Some(value.to_string()) // absolute (`Root.…`)
}

/// Whether an absolute reference path lands on a real component — itself, or an
/// implicit accessor child of one (so `Root.Foo.Value` resolves when `Root.Foo`
/// is a component). `pred` reports whether a candidate path is a known
/// component; the check walks the path and its ancestors.
pub fn reference_resolves(abs: &str, is_component: impl Fn(&str) -> bool) -> bool {
    if is_component(abs) {
        return true;
    }
    let mut p = abs;
    while let Some((head, _)) = p.rsplit_once('.') {
        if is_component(head) {
            return true;
        }
        p = head;
    }
    false
}

/// The `On <…>` clock leaves available under `Root.Events` (for an editor picker).
pub fn available_rates(xml: &str) -> Result<Vec<String>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out: Vec<String> = doc
        .descendants()
        .filter(is_real_component)
        .filter(|n| n.attribute("Classname") == Some("BuiltIn.EventKernel"))
        .filter_map(|n| n.attribute("Name"))
        .filter_map(|nm| nm.strip_prefix("Root.Events.On ").map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

/// The security groups a `set-security` / `create-*` value may legally use, for an
/// editor picker — the read-side parity for `list-rates`.
///
/// Security groups are **project-defined**: a project declares them inline in
/// `<SecurityMgr><SecurityRoles>` (the real AV-M1 project adds a custom `PDM`),
/// so an editor cannot hard-code the list. When the project declares roles, those
/// are returned in document order. When it has **no** `<SecurityMgr>` at all
/// (minimal/hand-written projects and the manual's "Automatic" tag-derived
/// security mode), the four documented standard groups ([`SECURITY_LEVELS`]) are
/// returned as the fallback.
///
/// This is the same resolution `validate_security` performs, so the picker and the
/// writer agree on one source of truth: every value this surfaces is one
/// `set_security` accepts, and nothing it omits is.
pub fn security_groups(xml: &str) -> Result<Vec<String>, EditError> {
    Ok(match declared_security_roles(xml)? {
        Some(roles) => roles,
        None => SECURITY_LEVELS.iter().map(|s| s.to_string()).collect(),
    })
}

// ---- shared helpers -------------------------------------------------------
