//! `m1-project` — structured, validated edits to a MoTeC M1 `Project.m1prj`.
//!
//! The `.m1prj` is a large XML file that MoTeC M1-Build also writes, so this tool
//! makes **surgical** edits: it locates the exact element with `roxmltree`
//! (byte-accurate) and splices the smallest possible text change, leaving the rest
//! of the file — formatting, comments, attribute order — byte-for-byte intact.
//! That keeps diffs reviewable and minimises clashes with M1-Build.
//!
//! Every mutation is a pure `&str -> Result<String, EditError>`, so it is trivial
//! to test and the CLI can offer `--dry-run`.
//!
//! Supported edits (see each `pub fn`):
//! - [`create_channel`] — add a `BuiltIn.Channel` component under an existing group.
//! - [`set_security`] — set/replace a component's `<Props Security="…">`.
//! - [`set_unit`] — set/replace a component's display unit (`<Locale><Default Unit>`).
//! - [`set_type`] — set/replace a component's storage `Type`.
//! - [`set_call_rate`] — point a script's `SelectedTrigger` at an `On <N>Hz` clock.

use std::fmt;

/// The MoTeC security / access levels, in increasing order of restriction. These
/// are the only values M1-Build accepts for `<Props Security="…">`.
pub const SECURITY_LEVELS: &[&str] = &["Tune", "Calibration", "Master Calibration", "Resource"];

/// Storage types a channel/parameter may declare (`<Props Type="…">`). Mirrors the
/// primitives the type checker understands; `bool` and the signed/unsigned widths.
pub const STORAGE_TYPES: &[&str] = &["bool", "u8", "u16", "u32", "s8", "s16", "s32", "f32", "f64"];

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    /// The file did not parse as XML.
    Xml(String),
    /// A referenced component path does not exist in the project.
    NoSuchComponent(String),
    /// The target name already exists (create would duplicate it).
    Duplicate(String),
    /// A value failed validation (unknown security level, type, rate, …).
    Invalid(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Xml(e) => write!(f, "invalid .m1prj XML: {e}"),
            EditError::NoSuchComponent(p) => write!(f, "no component named `{p}` in the project"),
            EditError::Duplicate(p) => write!(f, "a component named `{p}` already exists"),
            EditError::Invalid(m) => write!(f, "{m}"),
        }
    }
}
impl std::error::Error for EditError {}

/// A located component: its name, class, byte range of the whole `<Component>`
/// element, and the byte range of its `<Props>` child if present.
struct Located {
    classname: String,
    range: std::ops::Range<usize>,
    props_range: Option<std::ops::Range<usize>>,
}

/// Find a component by its fully-qualified `Name`, returning its layout.
fn locate(xml: &str, name: &str) -> Result<Located, EditError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))?;
    let node = doc
        .descendants()
        .find(|n| n.has_tag_name("Component") && n.attribute("Name") == Some(name))
        .ok_or_else(|| EditError::NoSuchComponent(name.to_string()))?;
    let props_range = node
        .children()
        .find(|c| c.has_tag_name("Props"))
        .map(|p| p.range());
    Ok(Located {
        classname: node.attribute("Classname").unwrap_or("").to_string(),
        range: node.range(),
        props_range,
    })
}

/// True if a component with this exact `Name` exists.
fn exists(xml: &str, name: &str) -> Result<bool, EditError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))?;
    Ok(doc
        .descendants()
        .any(|n| n.has_tag_name("Component") && n.attribute("Name") == Some(name)))
}

/// The leading whitespace (indentation) of the line containing byte `pos`.
fn indent_at(xml: &str, pos: usize) -> &str {
    let line_start = xml[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rest = &xml[line_start..];
    let end = rest
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The parent path of a dotted name (`Root.A.B` -> `Root.A`), or `None` for a
/// single segment.
fn parent_of(name: &str) -> Option<&str> {
    name.rfind('.').map(|i| &name[..i])
}

/// Create a `BuiltIn.Channel` component named `name` under its (existing) parent
/// group. `ty`/`unit`/`security` are optional. Inserted right after the last
/// existing component under the same parent (or after the parent itself), at the
/// parent's indentation.
pub fn create_channel(
    xml: &str,
    name: &str,
    ty: Option<&str>,
    unit: Option<&str>,
    security: Option<&str>,
) -> Result<String, EditError> {
    if let Some(t) = ty {
        validate_type(t)?;
    }
    if let Some(s) = security {
        validate_security(s)?;
    }
    if exists(xml, name)? {
        return Err(EditError::Duplicate(name.to_string()));
    }
    let parent = parent_of(name)
        .ok_or_else(|| EditError::Invalid(format!("`{name}` has no parent group")))?;
    let parent_loc = locate(xml, parent)?; // errors if the parent group is missing

    // Insert after the last existing component whose name is under `parent.`, so
    // siblings stay grouped; fall back to right after the parent element.
    let prefix = format!("{parent}.");
    let mut anchor_end = parent_loc.range.end;
    let mut anchor_for_indent = parent_loc.range.start;
    {
        let doc = roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))?;
        for n in doc.descendants().filter(|n| n.has_tag_name("Component")) {
            if let Some(nm) = n.attribute("Name")
                && nm.starts_with(&prefix)
                && n.range().end > anchor_end
            {
                anchor_end = n.range().end;
                anchor_for_indent = n.range().start;
            }
        }
    }
    let indent = indent_at(xml, anchor_for_indent).to_string();

    let props = build_props(ty, unit, security);
    let element = if props.is_empty() {
        format!(
            "\n{indent}<Component Classname=\"BuiltIn.Channel\" Name=\"{}\"/>",
            xml_escape(name)
        )
    } else {
        format!(
            "\n{indent}<Component Classname=\"BuiltIn.Channel\" Name=\"{}\">\n{indent} {props}\n{indent}</Component>",
            xml_escape(name)
        )
    };

    let mut out = String::with_capacity(xml.len() + element.len());
    out.push_str(&xml[..anchor_end]);
    out.push_str(&element);
    out.push_str(&xml[anchor_end..]);
    Ok(out)
}

/// Render the `<Props>` child for a new channel from the optional type/unit/security.
fn build_props(ty: Option<&str>, unit: Option<&str>, security: Option<&str>) -> String {
    let mut attrs = String::new();
    if let Some(t) = ty {
        attrs.push_str(&format!(" Type=\"{}\"", xml_escape(t)));
    }
    if let Some(s) = security {
        attrs.push_str(&format!(" Security=\"{}\"", xml_escape(s)));
    }
    match unit {
        Some(u) => format!(
            "<Props{attrs}><Locale><Default Unit=\"{}\"/></Locale></Props>",
            xml_escape(u)
        ),
        None if attrs.is_empty() => String::new(),
        None => format!("<Props{attrs}/>"),
    }
}

/// Set (or replace) a component's `<Props Security="…">`.
pub fn set_security(xml: &str, component: &str, security: &str) -> Result<String, EditError> {
    validate_security(security)?;
    set_props_attr(xml, component, "Security", security)
}

/// Set (or replace) a component's storage `Type`.
pub fn set_type(xml: &str, component: &str, ty: &str) -> Result<String, EditError> {
    validate_type(ty)?;
    set_props_attr(xml, component, "Type", ty)
}

/// Set (or replace) a component's display unit (`<Props><Locale><Default Unit>`).
pub fn set_unit(xml: &str, component: &str, unit: &str) -> Result<String, EditError> {
    // Ensure a <Props> exists, then set the Locale/Default unit inside it.
    let xml = ensure_props(xml, component)?;
    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    let props_text = &xml[props_range.clone()];

    let new_props = if let Some(u_range) = find_attr_value_range(props_text, "Unit") {
        // A Unit already exists somewhere in the Props subtree — replace its value.
        let abs = (props_range.start + u_range.start)..(props_range.start + u_range.end);
        return Ok(splice(&xml, abs, &xml_escape(unit)));
    } else if props_self_closing(props_text) {
        // `<Props …/>` -> `<Props …><Locale><Default Unit="…"/></Locale></Props>`.
        let open = props_text.trim_end();
        let open = open.strip_suffix("/>").unwrap();
        format!(
            "{open}><Locale><Default Unit=\"{}\"/></Locale></Props>",
            xml_escape(unit)
        )
    } else {
        // `<Props …> … </Props>` — insert the Locale just before `</Props>`.
        let close_idx = props_text
            .rfind("</Props>")
            .ok_or_else(|| EditError::Invalid("malformed <Props>".into()))?;
        format!(
            "{}<Locale><Default Unit=\"{}\"/></Locale>{}",
            &props_text[..close_idx],
            xml_escape(unit),
            &props_text[close_idx..]
        )
    };
    Ok(splice(&xml, props_range, &new_props))
}

/// Point a script (`FuncUser`/`MethodUser`) at the `On <rate>` clock by setting its
/// `<Props SelectedTrigger="…">`. `rate` is either `"startup"` (case-insensitive)
/// or a frequency in Hz (e.g. `100`). The matching `Root.Events.On <…>` kernel must
/// exist in the project.
pub fn set_call_rate(xml: &str, script: &str, rate: &str) -> Result<String, EditError> {
    let loc = locate(xml, script)?;
    if !loc.classname.contains("FuncUser") && !loc.classname.contains("MethodUser") {
        return Err(EditError::Invalid(format!(
            "`{script}` is a {} — only FuncUser/MethodUser scripts have a call rate",
            loc.classname
        )));
    }
    // The clock leaf: "On Startup" or "On <N>Hz".
    let leaf = if rate.eq_ignore_ascii_case("startup") {
        "On Startup".to_string()
    } else {
        let n = rate.trim().trim_end_matches("Hz").trim();
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(EditError::Invalid(format!(
                "rate must be `startup` or a number in Hz, got `{rate}`"
            )));
        }
        format!("On {n}Hz")
    };
    let clock = format!("Root.Events.{leaf}");
    if !exists(xml, &clock)? {
        let available = available_rates(xml)?;
        return Err(EditError::Invalid(format!(
            "no clock `{clock}` in the project; available: {}",
            available.join(", ")
        )));
    }
    // Trigger is group-relative: one `Parent.` per dot in the script's path lands
    // on `Root`, then `.Events.<leaf>` (every clock lives at `Root.Events.*`).
    let parents = "Parent.".repeat(script.matches('.').count());
    let trigger = format!("{parents}Events.{leaf}");
    set_props_attr(xml, script, "SelectedTrigger", &trigger)
}

/// The `On <…>` clock leaves available under `Root.Events` (for an editor picker).
pub fn available_rates(xml: &str) -> Result<Vec<String>, EditError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))?;
    let mut out: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component"))
        .filter(|n| n.attribute("Classname") == Some("BuiltIn.EventKernel"))
        .filter_map(|n| n.attribute("Name"))
        .filter_map(|nm| nm.strip_prefix("Root.Events.On ").map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

// ---- shared helpers -------------------------------------------------------

fn validate_security(s: &str) -> Result<(), EditError> {
    if SECURITY_LEVELS.contains(&s) {
        Ok(())
    } else {
        Err(EditError::Invalid(format!(
            "unknown security level `{s}`; valid: {}",
            SECURITY_LEVELS.join(", ")
        )))
    }
}

fn validate_type(t: &str) -> Result<(), EditError> {
    if STORAGE_TYPES.contains(&t) || t.starts_with("::") || t.contains('.') {
        // Primitives, or an enum reference (`::This.Foo`, `MoTeC Types.Bar`).
        Ok(())
    } else {
        Err(EditError::Invalid(format!(
            "unknown type `{t}`; valid primitives: {}",
            STORAGE_TYPES.join(", ")
        )))
    }
}

/// Set/replace an attribute on a component's `<Props>`, creating `<Props>` if absent.
fn set_props_attr(
    xml: &str,
    component: &str,
    attr: &str,
    value: &str,
) -> Result<String, EditError> {
    let xml = ensure_props(xml, component)?;
    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    // Only touch the opening tag of <Props> (up to the first '>').
    let open_end = xml[props_range.clone()]
        .find('>')
        .map(|i| props_range.start + i)
        .ok_or_else(|| EditError::Invalid("malformed <Props>".into()))?;
    let open_tag = &xml[props_range.start..open_end];

    if let Some(vr) = find_attr_value_range(open_tag, attr) {
        let abs = (props_range.start + vr.start)..(props_range.start + vr.end);
        Ok(splice(&xml, abs, &xml_escape(value)))
    } else {
        // Insert `attr="value"` just before the end of the opening tag, handling
        // a self-closing `<Props …/>` (drop the `/`).
        let insert_at = if open_tag.trim_end().ends_with('/') {
            xml[..open_end].rfind('/').unwrap()
        } else {
            open_end
        };
        let frag = format!(" {attr}=\"{}\"", xml_escape(value));
        Ok(splice(&xml, insert_at..insert_at, &frag))
    }
}

/// Ensure the component has a `<Props>` child; if it is `<Component …/>`
/// (self-closing) rewrite it to `<Component …><Props/></Component>`.
fn ensure_props(xml: &str, component: &str) -> Result<String, EditError> {
    let loc = locate(xml, component)?;
    if loc.props_range.is_some() {
        return Ok(xml.to_string());
    }
    let elem = &xml[loc.range.clone()];
    let indent = indent_at(xml, loc.range.start).to_string();
    if elem.trim_end().ends_with("/>") {
        let open = elem.trim_end().strip_suffix("/>").unwrap();
        let new = format!("{open}>\n{indent} <Props/>\n{indent}</Component>");
        Ok(splice(xml, loc.range, &new))
    } else {
        // Has children but no <Props>: insert <Props/> right after the open tag.
        let open_end = elem
            .find('>')
            .ok_or_else(|| EditError::Invalid("malformed <Component>".into()))?;
        let abs = loc.range.start + open_end + 1;
        let frag = format!("\n{indent} <Props/>");
        Ok(splice(xml, abs..abs, &frag))
    }
}

fn props_self_closing(props_text: &str) -> bool {
    props_text.trim_end().ends_with("/>")
}

/// Find the byte range of an attribute's *value* (between the quotes) within `s`.
/// Matches `attr="…"` with optional surrounding whitespace; returns the inner span.
fn find_attr_value_range(s: &str, attr: &str) -> Option<std::ops::Range<usize>> {
    let mut from = 0;
    while let Some(i) = s[from..].find(attr) {
        let at = from + i;
        // Must be a whole attribute name: preceded by whitespace/'<', followed by `="`.
        let before_ok =
            at == 0 || s.as_bytes()[at - 1].is_ascii_whitespace() || s.as_bytes()[at - 1] == b'<';
        let after = &s[at + attr.len()..];
        let after_trim = after.trim_start();
        if before_ok && after_trim.starts_with('=') {
            let eq = after.find('=').unwrap();
            let rest = &after[eq + 1..];
            let q = rest.find('"')?;
            let val_start = at + attr.len() + eq + 1 + q + 1;
            let val_end = val_start + s[val_start..].find('"')?;
            return Some(val_start..val_end);
        }
        from = at + attr.len();
    }
    None
}

fn splice(s: &str, range: std::ops::Range<usize>, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len() - (range.end - range.start) + replacement.len());
    out.push_str(&s[..range.start]);
    out.push_str(replacement);
    out.push_str(&s[range.end..]);
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRJ: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="T">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Engine"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Engine.Speed">
     <Props Type="f32" Security="Tune"/>
    </Component>
    <Component Classname="BuiltIn.Channel" Name="Root.Engine.Plain"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On Startup"/>
    <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Update"/>
    <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Sub.Tick"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

    fn parses(xml: &str) {
        roxmltree::Document::parse(xml).expect("result must be valid XML");
    }

    #[test]
    fn create_channel_inserts_under_group() {
        let out = create_channel(
            PRJ,
            "Root.Engine.Torque",
            Some("f32"),
            Some("N.m"),
            Some("Tune"),
        )
        .unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.Engine.Torque""#));
        assert!(out.contains(r#"Type="f32""#));
        assert!(out.contains(r#"Unit="N.m""#));
        assert!(out.contains(r#"Security="Tune""#));
        // The new channel sits after the existing Root.Engine.* components.
        let torque = out.find("Root.Engine.Torque").unwrap();
        let plain = out.find("Root.Engine.Plain").unwrap();
        assert!(
            torque > plain,
            "new channel should follow existing siblings"
        );
    }

    #[test]
    fn create_channel_rejects_duplicate() {
        assert_eq!(
            create_channel(PRJ, "Root.Engine.Speed", None, None, None),
            Err(EditError::Duplicate("Root.Engine.Speed".into()))
        );
    }

    #[test]
    fn create_channel_rejects_missing_parent() {
        assert!(matches!(
            create_channel(PRJ, "Root.Ghost.X", None, None, None),
            Err(EditError::NoSuchComponent(_))
        ));
    }

    #[test]
    fn create_channel_rejects_bad_security() {
        assert!(matches!(
            create_channel(PRJ, "Root.Engine.X", None, None, Some("Wizard")),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn set_security_replaces_existing() {
        let out = set_security(PRJ, "Root.Engine.Speed", "Calibration").unwrap();
        parses(&out);
        assert!(out.contains(r#"Security="Calibration""#));
        assert!(!out.contains(r#"Security="Tune""#));
        // The Type attribute is untouched.
        assert!(out.contains(r#"Type="f32""#));
    }

    #[test]
    fn set_security_adds_to_self_closing_props_component() {
        // Root.Engine.Plain is `<Component …/>` with no Props.
        let out = set_security(PRJ, "Root.Engine.Plain", "Resource").unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.Engine.Plain""#));
        assert!(out.contains(r#"Security="Resource""#));
    }

    #[test]
    fn set_type_sets_and_replaces() {
        let out = set_type(PRJ, "Root.Engine.Plain", "u16").unwrap();
        parses(&out);
        assert!(out.contains(r#"Type="u16""#));
        let out2 = set_type(&out, "Root.Engine.Plain", "s32").unwrap();
        assert!(out2.contains(r#"Type="s32""#) && !out2.contains(r#"Type="u16""#));
    }

    #[test]
    fn set_unit_on_props_without_locale() {
        let out = set_unit(PRJ, "Root.Engine.Speed", "rpm").unwrap();
        parses(&out);
        assert!(out.contains(r#"<Default Unit="rpm"/>"#));
        // Replacing keeps a single Unit.
        let out2 = set_unit(&out, "Root.Engine.Speed", "rad/s").unwrap();
        parses(&out2);
        assert!(out2.contains(r#"Unit="rad/s""#) && !out2.contains(r#"Unit="rpm""#));
        assert_eq!(out2.matches("Unit=").count(), 1);
    }

    #[test]
    fn set_call_rate_builds_relative_trigger() {
        // Root.Engine.Update -> 2 dots -> "Parent.Parent.Events.On 100Hz".
        let out = set_call_rate(PRJ, "Root.Engine.Update", "100").unwrap();
        parses(&out);
        assert!(out.contains(r#"SelectedTrigger="Parent.Parent.Events.On 100Hz""#));
    }

    #[test]
    fn set_call_rate_depth_scales_parents() {
        // Root.Engine.Sub.Tick -> 3 dots -> 3 Parents.
        let out = set_call_rate(PRJ, "Root.Engine.Sub.Tick", "100Hz").unwrap();
        assert!(out.contains(r#"SelectedTrigger="Parent.Parent.Parent.Events.On 100Hz""#));
    }

    #[test]
    fn set_call_rate_startup() {
        let out = set_call_rate(PRJ, "Root.Engine.Update", "startup").unwrap();
        assert!(out.contains(r#"SelectedTrigger="Parent.Parent.Events.On Startup""#));
    }

    #[test]
    fn set_call_rate_rejects_missing_clock() {
        let err = set_call_rate(PRJ, "Root.Engine.Update", "999").unwrap_err();
        assert!(matches!(err, EditError::Invalid(_)));
    }

    #[test]
    fn set_call_rate_rejects_non_script() {
        assert!(matches!(
            set_call_rate(PRJ, "Root.Engine.Speed", "100"),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn available_rates_lists_clocks() {
        let rates = available_rates(PRJ).unwrap();
        assert!(rates.contains(&"100Hz".to_string()));
        assert!(rates.contains(&"Startup".to_string()));
    }
}
