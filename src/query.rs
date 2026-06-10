//! Read-only queries: enumerate components (`list_components`), the call-rate
//! catalogue (`available_rates`) and group-relative trigger resolution
//! (`resolve_trigger`).

use crate::EditError;
use crate::xml::*;

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
    /// Depth in the path hierarchy (number of dots in `path`).
    pub depth: usize,
}

/// Enumerate all `<Component>` elements in document order.
///
/// Only real components (those with a `Classname` attribute) are returned.
/// The `<Organisation>` section of a real `.m1prj` also contains `<Component>` nodes
/// without `Classname` — those are view-only structural nodes and are excluded.
pub fn list_components(xml: &str) -> Result<Vec<ComponentEntry>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out = Vec::new();
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
    {
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
        let depth = path.matches('.').count();
        out.push(ComponentEntry {
            path: path.to_string(),
            classname,
            ty,
            unit,
            security,
            call_rate,
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
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
    {
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

/// The `On <…>` clock leaves available under `Root.Events` (for an editor picker).
pub fn available_rates(xml: &str) -> Result<Vec<String>, EditError> {
    let doc = parse_xml(xml)?;
    let mut out: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter(|n| n.attribute("Classname") == Some("BuiltIn.EventKernel"))
        .filter_map(|n| n.attribute("Name"))
        .filter_map(|nm| nm.strip_prefix("Root.Events.On ").map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

// ---- shared helpers -------------------------------------------------------
