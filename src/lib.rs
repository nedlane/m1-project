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
//! - [`create_group`] — add a `BuiltIn.GroupCompound` under an existing group.
//! - [`delete_component`] — remove a component element (and optionally its subtree).
//! - [`rename_component`] — rename a component and update all `SelectedTrigger` references.
//! - [`set_security`] — set/replace a component's `<Props Security="…">`.
//! - [`set_unit`] — set/replace a component's display unit (`<Locale><Default Unit>`).
//! - [`set_type`] — set/replace a component's storage `Type`.
//! - [`set_call_rate`] — point a script's `SelectedTrigger` at an `On <N>Hz` clock.
//! - [`validate`] — read-only check for structural violations.
//! - [`list_components`] — enumerate every component in document order.

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

// LIFETIME NOTE: a `roxmltree::Node<'a, 'd>` borrows BOTH the parsed `Document`
// (lifetime `'d`, the arena) and the source `&str` (`'a`). The `Document` value
// itself is a local — so a helper can NOT return `(Document, Node)` together: the
// returned `Node` would borrow a `Document` that is being moved out at the same
// time (self-referential; the borrow checker rejects it with E0505/E0515). That
// is *why* these helpers re-parse the (cheap) `&str` at each site and keep every
// `Document` local to its caller:
//   * `parse_xml` does the bare parse-and-map-error, shared by the scan helpers
//     that walk all components (`exists`, `available_rates`, `create_channel`).
//   * `parse_and_find` finds one `<Component>` in an *already-parsed* `doc` the
//     CALLER owns, so the `doc` binding outlives the borrowed `Node`.
// Do NOT "optimise" this into a single helper that owns the `Document` and hands
// back a `Node`: it cannot compile, and the rebinds here (`let xml =
// ensure_props(...)?;`) would only make the lifetime tangle worse.

/// Parse `xml` into a `roxmltree::Document`, mapping a parse failure to
/// [`EditError::Xml`] with the same message every call site used.
fn parse_xml(xml: &str) -> Result<roxmltree::Document<'_>, EditError> {
    roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))
}

/// Find the `<Component>` whose `Name` is `component` in an already-parsed `doc`,
/// erroring with [`EditError::NoSuchComponent`] if absent. The caller owns `doc`
/// so the returned `Node` (which borrows it) stays valid — see the LIFETIME NOTE
/// above for why the `Document` is not created/returned here.
fn parse_and_find<'a, 'd>(
    doc: &'d roxmltree::Document<'a>,
    component: &str,
) -> Result<roxmltree::Node<'d, 'a>, EditError> {
    doc.descendants()
        .find(|n| {
            n.has_tag_name("Component")
                && n.has_attribute("Classname")
                && n.attribute("Name") == Some(component)
        })
        .ok_or_else(|| EditError::NoSuchComponent(component.to_string()))
}

/// A located component: its name, class, byte range of the whole `<Component>`
/// element, and the byte range of its `<Props>` child if present.
struct Located {
    classname: String,
    range: std::ops::Range<usize>,
    props_range: Option<std::ops::Range<usize>>,
}

/// Find a component by its fully-qualified `Name`, returning its layout.
fn locate(xml: &str, name: &str) -> Result<Located, EditError> {
    let doc = parse_xml(xml)?;
    let node = parse_and_find(&doc, name)?;
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

/// True if a component with this exact `Name` exists (only considers real components
/// that carry a `Classname` attribute, excluding `<Organisation>` view-only nodes).
fn exists(xml: &str, name: &str) -> Result<bool, EditError> {
    let doc = parse_xml(xml)?;
    Ok(doc.descendants().any(|n| {
        n.has_tag_name("Component")
            && n.has_attribute("Classname")
            && n.attribute("Name") == Some(name)
    }))
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
        let doc = parse_xml(xml)?;
        for n in doc
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            // Anchor on the last *direct* child of `parent` — one whose Name is
            // exactly one segment deeper. `starts_with(&prefix)` alone also
            // matches a grandchild like `Root.Engine.Sub.Deep`, which would drop
            // the new sibling after a sub-group's components — and, in nested
            // layouts, after the parent group's closing tags, outside it (#8).
            if let Some(nm) = n.attribute("Name")
                && let Some(rest) = nm.strip_prefix(&prefix)
                && !rest.is_empty()
                && !rest.contains('.')
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

/// Create a `BuiltIn.GroupCompound` component named `name` under its (existing) parent
/// group.  Mirrors [`create_channel`]'s anchor/splice logic — inserted right after the
/// last existing direct child of the parent (or after the parent itself).
pub fn create_group(xml: &str, name: &str) -> Result<String, EditError> {
    validate_name_segment(name)?;
    if exists(xml, name)? {
        return Err(EditError::Duplicate(name.to_string()));
    }
    let parent = parent_of(name)
        .ok_or_else(|| EditError::Invalid(format!("`{name}` has no parent group")))?;
    let parent_loc = locate(xml, parent)?;

    let prefix = format!("{parent}.");
    let mut anchor_end = parent_loc.range.end;
    let mut anchor_for_indent = parent_loc.range.start;
    {
        let doc = parse_xml(xml)?;
        for n in doc
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            if let Some(nm) = n.attribute("Name")
                && let Some(rest) = nm.strip_prefix(&prefix)
                && !rest.is_empty()
                && !rest.contains('.')
                && n.range().end > anchor_end
            {
                anchor_end = n.range().end;
                anchor_for_indent = n.range().start;
            }
        }
    }
    let indent = indent_at(xml, anchor_for_indent).to_string();
    let element = format!(
        "\n{indent}<Component Classname=\"BuiltIn.GroupCompound\" Name=\"{}\"/>",
        xml_escape(name)
    );
    let mut out = String::with_capacity(xml.len() + element.len());
    out.push_str(&xml[..anchor_end]);
    out.push_str(&element);
    out.push_str(&xml[anchor_end..]);
    Ok(out)
}

/// Remove a component (and its whole subtree of components whose names start with
/// `name.`) from the project XML.
///
/// - If the component has direct children and `recursive` is `false`, returns
///   [`EditError::Invalid`].
/// - If any component's `SelectedTrigger` references a path that starts with
///   `name` (direct or descendant), refuse unless `force` is `true` — when
///   `force` is `false` the error message lists the referencing names.
/// - Also removes the preceding indentation whitespace so the file stays tidy.
pub fn delete_component(
    xml: &str,
    name: &str,
    recursive: bool,
    force: bool,
) -> Result<String, EditError> {
    // Locate the target first so we get a clean NoSuchComponent error.
    let _loc = locate(xml, name)?;

    let doc = parse_xml(xml)?;

    // Direct children: names that are exactly one segment deeper.
    let prefix = format!("{name}.");
    let has_children = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .any(|n| {
            n.attribute("Name")
                .and_then(|nm| nm.strip_prefix(&prefix))
                .map(|rest| !rest.contains('.'))
                == Some(true)
        });

    if has_children && !recursive {
        return Err(EditError::Invalid(format!(
            "`{name}` has child components — pass --recursive to delete the whole subtree"
        )));
    }

    // Referencing check: SelectedTrigger is group-relative; a reference to
    // `name` or a descendant resolves via the parent chain to that path.
    // Rather than re-resolving relative triggers (expensive), we check whether
    // the absolute path `name` itself or any descendant name is the target of a
    // trigger by looking for components that *use* this component as their clock
    // source — but triggers point at `Root.Events.*`, not at regular components.
    // The only cross-component path reference relevant to deletion is a component
    // whose SelectedTrigger resolves to a clock that is being deleted.
    //
    // More practically: if a group named e.g. `Root.Events` is deleted and it
    // contains an `On 100Hz` kernel, any script pointing at that kernel would
    // break.  We check for that by collecting all clock absolute paths under
    // `name` and seeing if any SelectedTrigger resolves to one of them.
    //
    // For ordinary (non-Events) components: no other component field holds an
    // absolute path to another component (confirmed by corpus grep — only
    // `SelectedTrigger` and even that is group-relative pointing at Events).
    let deleted_names: std::collections::HashSet<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter_map(|n| n.attribute("Name"))
        .filter(|nm| *nm == name || nm.starts_with(&prefix))
        .map(str::to_string)
        .collect();

    // Build absolute paths for all clock components under the deleted set.
    let deleted_clocks: std::collections::HashSet<String> = deleted_names
        .iter()
        .filter(|nm| {
            doc.descendants()
                .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
                .find(|n| n.attribute("Name") == Some(nm.as_str()))
                .map(|n| n.attribute("Classname") == Some("BuiltIn.EventKernel"))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    // For every component NOT being deleted, resolve its SelectedTrigger and
    // see if it lands on a deleted clock.
    let mut referencing: Vec<String> = Vec::new();
    if !deleted_clocks.is_empty() {
        for n in doc
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
            .filter(|n| n.attribute("Name").map(|nm| !deleted_names.contains(nm)) == Some(true))
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
            // Resolve the group-relative trigger to an absolute path.
            if let Some(abs) = resolve_trigger(owner, trigger)
                && deleted_clocks.contains(&abs)
            {
                referencing.push(owner.to_string());
            }
        }
    }

    if !referencing.is_empty() && !force {
        referencing.sort();
        return Err(EditError::Invalid(format!(
            "cannot delete `{name}`: referenced by SelectedTrigger in: {}; pass --force to delete anyway",
            referencing.join(", ")
        )));
    }

    // Collect the byte range of every element to remove (target + all descendants
    // in `deleted_names`), then splice them out right-to-left so earlier ranges
    // don't invalidate later byte offsets.
    //
    // IMPORTANT: descendants may not be contiguous in the serialised XML (a
    // real .m1prj lists components in a flat `<List>` with siblings of other
    // subtrees interspersed).  A single min→max span would accidentally erase
    // those intervening siblings, so each element is removed individually.
    let mut ranges_to_remove: Vec<std::ops::Range<usize>> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter(|n| n.attribute("Name").map(|nm| deleted_names.contains(nm)) == Some(true))
        .map(|n| {
            // Extend the range backwards over the preceding indentation AND its
            // line break (LF or CRLF) so the deleted element's whole line goes,
            // leaving no blank line behind.
            let start = n.range().start;
            let before = &xml[..start];
            let ws_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let prefix_is_ws = xml[ws_start..start].chars().all(|c| c == ' ' || c == '\t');
            let actual_start = if prefix_is_ws && ws_start > 0 {
                // ws_start is just past a '\n'; also consume a preceding '\r'.
                if xml[..ws_start - 1].ends_with('\r') {
                    ws_start - 2
                } else {
                    ws_start - 1
                }
            } else if prefix_is_ws {
                ws_start
            } else {
                start
            };
            actual_start..n.range().end
        })
        .collect();

    // Sort descending by start so right-to-left application keeps offsets valid.
    ranges_to_remove.sort_by_key(|r| std::cmp::Reverse(r.start));

    let mut result = xml.to_string();
    for range in ranges_to_remove {
        result = splice(&result, range, "");
    }
    Ok(result)
}

/// Rename a component, updating its `Name` attribute and every `SelectedTrigger`
/// in the file whose resolved absolute path matches the old name or any of its
/// descendants.
///
/// `new_segment` must be a single identifier (no dots). Returns the rewritten XML
/// and a list of `.m1scr` backing file names that the caller may need to rename
/// on disk (the tool does NOT rename files).
pub fn rename_component(
    xml: &str,
    old_name: &str,
    new_segment: &str,
) -> Result<(String, Vec<String>), EditError> {
    validate_name_segment(new_segment)?;

    // Compute what the new full name will be.
    let new_name = match parent_of(old_name) {
        Some(parent) => format!("{parent}.{new_segment}"),
        None => new_segment.to_string(),
    };

    if new_name == old_name {
        return Ok((xml.to_string(), Vec::new()));
    }

    // Verify old_name exists and new_name does not.
    if !exists(xml, old_name)? {
        return Err(EditError::NoSuchComponent(old_name.to_string()));
    }
    if exists(xml, &new_name)? {
        return Err(EditError::Duplicate(new_name));
    }

    let old_prefix = format!("{old_name}.");
    let doc = parse_xml(xml)?;

    // Collect every component name that will be renamed (old_name and all descendants).
    let mut to_rename: Vec<(String, String)> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .filter_map(|n| n.attribute("Name"))
        .filter(|nm| *nm == old_name || nm.starts_with(&old_prefix))
        .map(|nm| {
            let new = if nm == old_name {
                new_name.clone()
            } else {
                format!("{new_name}.{}", &nm[old_name.len() + 1..])
            };
            (nm.to_string(), new)
        })
        .collect();
    // Sort by document position (old names are unique so sort by old name length desc
    // then alpha to ensure longest match first when we splice).
    to_rename.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));

    // Collect backing script filenames to warn about.
    // Convention: `Root.X.Y.Z` → `X.Y.Z.m1scr` (everything after `Root.`).
    let script_warnings: Vec<String> = to_rename
        .iter()
        .filter(|(old, _)| {
            doc.descendants()
                .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
                .find(|n| n.attribute("Name") == Some(old.as_str()))
                .map(|n| {
                    n.attribute("Classname")
                        .map(|c| c.contains("FuncUser") || c.contains("MethodUser"))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .map(|(_, new)| {
            // Convention: `Root.X.Y.Z` → `X.Y.Z.m1scr`
            // (drop the `Root.` prefix; keep dots; append `.m1scr`)
            let suffix = new.strip_prefix("Root.").unwrap_or(new.as_str());
            format!("{suffix}.m1scr")
        })
        .collect();

    // Now rewrite the XML.  We collect all Name attribute value ranges plus all
    // SelectedTrigger value ranges that need rewriting, then apply the splices
    // from right to left (highest offset first) so earlier splices don't
    // invalidate later byte offsets.
    //
    // Build a fresh parse (the above `doc` borrow is about to be consumed by the
    // new parse we need for offset collection).
    drop(doc);

    // We'll do multiple passes: first rename all Name attrs, then fix SelectedTriggers.
    // Because we're building a new string each pass, byte offsets stay valid within
    // each pass if we apply splices right-to-left.

    // Pass 1: rename Name attributes (largest-offset first).
    let mut result = xml.to_string();
    {
        let doc2 = parse_xml(&result)?;
        // Collect (offset, old_val, new_val) for each Name attr that needs changing.
        let mut renames: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for n in doc2
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            let Some(nm) = n.attribute("Name") else {
                continue;
            };
            if let Some((_, new)) = to_rename.iter().find(|(old, _)| old == nm) {
                let attr = n.attribute_node("Name").unwrap();
                renames.push((attr.range_value(), new.clone()));
            }
        }
        renames.sort_by_key(|r| std::cmp::Reverse(r.0.start));
        for (range, new_val) in renames {
            result = splice(&result, range, &xml_escape(&new_val));
        }
    }

    // Pass 2: fix SelectedTrigger references.
    // A SelectedTrigger is group-relative from the owning script.  We need to
    // find ones that resolve (via resolve_trigger) to an absolute path that
    // starts with old_name (with or without trailing dot).
    {
        let doc3 = parse_xml(&result)?;
        let mut trigger_fixes: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for n in doc3
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            // After pass 1, the owner's Name is already the NEW name.
            let Some(owner_new) = n.attribute("Name") else {
                continue;
            };
            // Get the OLD owner name (we renamed it, so reverse-map).
            let owner_old = to_rename
                .iter()
                .find(|(_, new)| new == owner_new)
                .map(|(old, _)| old.as_str())
                .unwrap_or(owner_new);

            let Some(props) = n.children().find(|c| c.has_tag_name("Props")) else {
                continue;
            };
            let Some(trigger_attr) = props.attribute_node("SelectedTrigger") else {
                continue;
            };
            let trigger_val = props.attribute("SelectedTrigger").unwrap();
            // Resolve using the OLD owner name.
            let Some(abs) = resolve_trigger(owner_old, trigger_val) else {
                continue;
            };
            // Does the absolute path match old_name or a descendant?
            let new_abs = if abs == old_name {
                new_name.clone()
            } else if let Some(rest) = abs.strip_prefix(&old_prefix) {
                format!("{new_name}.{rest}")
            } else {
                continue; // not affected
            };
            // Rebuild the trigger as group-relative from the NEW owner name.
            let new_trigger = build_trigger(owner_new, &new_abs);
            trigger_fixes.push((trigger_attr.range_value(), new_trigger));
        }
        trigger_fixes.sort_by_key(|r| std::cmp::Reverse(r.0.start));
        for (range, new_val) in trigger_fixes {
            result = splice(&result, range, &xml_escape(&new_val));
        }
    }

    Ok((result, script_warnings))
}

// ---- validate ---------------------------------------------------------------

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

/// Build a group-relative `SelectedTrigger` value for `owner` pointing at
/// `abs_clock` (an absolute component path).
fn build_trigger(owner: &str, abs_clock: &str) -> String {
    let owner_segs: Vec<&str> = owner.split('.').collect();
    let clock_segs: Vec<&str> = abs_clock.split('.').collect();
    // Find the longest common ancestor prefix.
    let common = owner_segs
        .iter()
        .zip(clock_segs.iter())
        .take_while(|(a, b)| a == b)
        .count();
    // Number of times we climb from owner to the common ancestor.
    // owner has `owner_segs.len()` segments; we need to climb to `common` depth.
    // That means popping `owner_segs.len() - common` levels.
    let climb = owner_segs.len() - common;
    let parents = "Parent.".repeat(climb);
    let tail = clock_segs[common..].join(".");
    format!("{parents}{tail}")
}

/// Validate that `name` is a valid component name (no dots; allowed chars: letters,
/// digits, space, underscore, hyphen, parentheses — matching M1 naming convention).
fn validate_name_segment(name: &str) -> Result<(), EditError> {
    // Reject empty or dot-containing names (dots delimit path segments).
    if name.is_empty() {
        return Err(EditError::Invalid("name must not be empty".into()));
    }
    // The full dotted path form is allowed for create_group (caller passes the
    // FULL path like `Root.Engine.NewGroup`).  We validate that the LAST segment
    // (after the last dot) has no further dots.  That last segment is the new
    // component's Name attribute value.
    let segment = name.rsplit('.').next().unwrap_or(name);
    if segment.is_empty() {
        return Err(EditError::Invalid("name must not end with a dot".into()));
    }
    // M1 names: letters, digits, underscore, hyphen, space, parentheses are all
    // seen in the corpus.  The real constraint from the manual is "no dot in a
    // segment" (dots are reserved for path separation).  We enforce that only.
    if segment.contains('.') {
        return Err(EditError::Invalid(format!(
            "name segment `{segment}` must not contain a dot"
        )));
    }
    Ok(())
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

    // Replace the value of the *real* display unit — the `Unit` attribute on the
    // `<Default>` element — located via the XML parser. A plain text scan of the
    // whole <Props> subtree would also match a `Unit="…"` in a comment or an
    // unrelated child element and mutate that instead, silently (#7).
    if let Some(u_range) = default_unit_value_range(&xml, component)? {
        return Ok(splice(&xml, u_range, &xml_escape(unit)));
    }

    // A `<Default>` may already exist carrying other attributes but no `Unit`
    // (the corpus is full of `<Default DPS="3"/>` / `<Default Min Max/>`). Add
    // the `Unit` to *that* element rather than appending a whole second
    // `<Locale>`, which would duplicate the element and orphan its DPS/format.
    if let Some(insert_at) = default_unit_insert_point(&xml, component)? {
        let frag = format!(" Unit=\"{}\"", xml_escape(unit));
        return Ok(splice(&xml, insert_at..insert_at, &frag));
    }

    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    let props_text = &xml[props_range.clone()];

    let new_props = if props_self_closing(props_text) {
        // `<Props …/>` -> `<Props …><Locale><Default Unit="…"/></Locale></Props>`.
        let open = props_text.trim_end();
        let open = open
            .strip_suffix("/>")
            .expect("checked by props_self_closing above");
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

    // Replace the attribute's value, located precisely via the XML parser so the
    // range is the real value and never a false text match inside another
    // attribute's quoted value (which spliced across attribute boundaries and
    // wrote not-well-formed XML while reporting success) (#5).
    if let Some(vr) = props_attr_value_range(&xml, component, attr)? {
        return Ok(splice(&xml, vr, &xml_escape(value)));
    }

    // Attribute absent: insert ` attr="value"` immediately after the `<Props`
    // tag name. Valid whether or not the tag already has attributes and whether
    // it is self-closing (`<Props/>` -> `<Props attr="…"/>`).
    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    let insert_at = props_range.start + "<Props".len();
    let frag = format!(" {attr}=\"{}\"", xml_escape(value));
    Ok(splice(&xml, insert_at..insert_at, &frag))
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
        let open = elem
            .trim_end()
            .strip_suffix("/>")
            .expect("checked by ends_with(\"/>\") above");
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

/// The byte range (in `xml`) of the value of `attr` on the target component's
/// `<Props>` opening tag, located via the XML parser. `roxmltree`'s
/// `range_value()` is the exact span between the quotes, so the replace can never
/// land inside another attribute's quoted value (#5).
fn props_attr_value_range(
    xml: &str,
    component: &str,
    attr: &str,
) -> Result<Option<std::ops::Range<usize>>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|p| p.attribute_node(attr))
        .map(|a| a.range_value()))
}

/// The byte range of the `Unit` value on this component's `<Default>` element —
/// the real display unit (`<Props><Locale><Default Unit="…"/>`) — located via the
/// XML parser so a `Unit="…"` in a comment or a non-`Default` element is ignored
/// rather than mutated (#7).
fn default_unit_value_range(
    xml: &str,
    component: &str,
) -> Result<Option<std::ops::Range<usize>>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|props| {
            props
                .descendants()
                .find(|d| d.has_tag_name("Default") && d.has_attribute("Unit"))
        })
        .and_then(|d| d.attribute_node("Unit"))
        .map(|a| a.range_value()))
}

/// The byte offset just after the `<Default` tag name of this component's
/// existing `<Props><Locale><Default …>` element that has **no** `Unit`
/// attribute yet — the point to splice ` Unit="…"` into. `None` if there is no
/// such `<Default>` (so the caller falls back to creating the whole `<Locale>`).
fn default_unit_insert_point(xml: &str, component: &str) -> Result<Option<usize>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|props| {
            props
                .descendants()
                .find(|d| d.has_tag_name("Default") && !d.has_attribute("Unit"))
        })
        // node.range() spans the whole element; insert right after `<Default`.
        .map(|d| d.range().start + "<Default".len()))
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
    <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Update"/>
    <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Sub.Tick"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Events"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On Startup"/>
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
    fn set_unit_adds_unit_to_existing_default_without_unit() {
        // The corpus is full of `<Default DPS="3"/>` / `<Default Min Max/>` —
        // a <Locale><Default> that carries other attributes but no Unit yet.
        // set_unit must add the Unit *to that Default*, not append a second
        // <Locale> (which duplicates the element and drops the existing DPS).
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.Channel" Name="Root.Y"><Props Type="f32"><Locale><Default DPS="3"/></Locale></Props></Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let out = set_unit(prj, "Root.Y", "rpm").unwrap();
        parses(&out);
        assert_eq!(
            out.matches("<Locale>").count(),
            1,
            "must reuse the existing <Locale>, not append a second one"
        );
        assert_eq!(out.matches("<Default").count(), 1, "single <Default>");
        assert!(out.contains(r#"DPS="3""#), "existing DPS must be preserved");
        assert!(out.contains(r#"Unit="rpm""#), "Unit added to the Default");
        // Idempotent / replaceable: a second set_unit just swaps the value.
        let out2 = set_unit(&out, "Root.Y", "rad/s").unwrap();
        parses(&out2);
        assert_eq!(out2.matches("Unit=").count(), 1);
        assert!(out2.contains(r#"DPS="3""#));
        assert!(out2.contains(r#"Unit="rad/s""#) && !out2.contains(r#"Unit="rpm""#));
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

    #[test]
    fn set_props_attr_ignores_match_inside_another_attr_value() {
        // #5: an earlier attribute's value contains ` Type=`. The replace must
        // target the real `Type` attribute, not splice across the Validation
        // value's closing quote — which produced not-well-formed XML.
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.Channel" Name="Root.X"><Props Validation="if Type=invalid reject" Type="f32"/></Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let out = set_type(prj, "Root.X", "u16").unwrap();
        parses(&out); // old code wrote `...reject"u16"f32"` — not well-formed
        assert!(out.contains(r#"Type="u16""#));
        assert!(out.contains(r#"Validation="if Type=invalid reject""#)); // untouched
        assert!(!out.contains(r#"Type="f32""#));
    }

    #[test]
    fn set_unit_targets_default_not_comment_or_sibling() {
        // #7: a `Unit="…"` in a comment (and a non-Default sibling) must be
        // ignored; only the real `<Default Unit>` display unit is replaced.
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.Channel" Name="Root.Y"><Props><!-- legacy Unit="deprecated" --><Meta Unit="bogus"/><Locale><Default Unit="rpm"/></Locale></Props></Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let out = set_unit(prj, "Root.Y", "rad/s").unwrap();
        parses(&out);
        assert!(out.contains(r#"<Default Unit="rad/s"/>"#)); // the real unit changed
        assert!(out.contains(r#"Unit="deprecated""#)); // comment untouched
        assert!(out.contains(r#"<Meta Unit="bogus"/>"#)); // sibling untouched
    }

    #[test]
    fn create_channel_anchors_on_direct_child_not_grandchild() {
        // #8: Root.Engine has direct children (…Update) and a grandchild
        // (Root.Engine.Sub.Tick). The new sibling must land after the last
        // DIRECT child, not after the grandchild.
        let out = create_channel(PRJ, "Root.Engine.New", None, None, None).unwrap();
        parses(&out);
        let newc = out.find(r#"Name="Root.Engine.New""#).unwrap();
        let update = out.find(r#"Name="Root.Engine.Update""#).unwrap();
        let grandchild = out.find(r#"Name="Root.Engine.Sub.Tick""#).unwrap();
        assert!(
            newc > update,
            "new channel should follow the last direct child"
        );
        assert!(
            newc < grandchild,
            "new channel must not be placed after the grandchild Sub.Tick"
        );
    }

    // ---- #21 create_group ---------------------------------------------------

    #[test]
    fn create_group_inserts_under_parent() {
        let out = create_group(PRJ, "Root.Engine.SubSystem").unwrap();
        parses(&out);
        assert!(out.contains(r#"Classname="BuiltIn.GroupCompound" Name="Root.Engine.SubSystem""#));
        // Placed after the last direct child of Root.Engine.
        let new_pos = out.find("Root.Engine.SubSystem").unwrap();
        let update_pos = out.find("Root.Engine.Update").unwrap();
        assert!(
            new_pos > update_pos,
            "new group should follow the last direct child"
        );
    }

    #[test]
    fn create_group_rejects_duplicate() {
        assert_eq!(
            create_group(PRJ, "Root.Engine"),
            Err(EditError::Duplicate("Root.Engine".into()))
        );
    }

    #[test]
    fn create_group_rejects_missing_parent() {
        assert!(matches!(
            create_group(PRJ, "Root.Ghost.Sub"),
            Err(EditError::NoSuchComponent(_))
        ));
    }

    #[test]
    fn create_group_under_root() {
        let out = create_group(PRJ, "Root.NewTop").unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.NewTop""#));
    }

    // ---- #22 delete_component -----------------------------------------------

    #[test]
    fn delete_leaf_component() {
        let out = delete_component(PRJ, "Root.Engine.Plain", false, false).unwrap();
        parses(&out);
        assert!(!out.contains(r#"Name="Root.Engine.Plain""#));
        // Other components untouched.
        assert!(out.contains(r#"Name="Root.Engine.Speed""#));
    }

    #[test]
    fn delete_group_without_recursive_fails() {
        assert!(matches!(
            delete_component(PRJ, "Root.Engine", false, false),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn delete_group_recursive_removes_subtree() {
        let out = delete_component(PRJ, "Root.Engine", true, false).unwrap();
        parses(&out);
        assert!(!out.contains("Root.Engine"));
        // Events untouched.
        assert!(out.contains("Root.Events"));
    }

    #[test]
    fn delete_missing_component_errors() {
        assert!(matches!(
            delete_component(PRJ, "Root.Ghost", false, false),
            Err(EditError::NoSuchComponent(_))
        ));
    }

    #[test]
    fn delete_events_group_blocked_by_references() {
        // Root.Events contains On 100Hz; Root.Engine.Update and Sub.Tick both
        // point at it via SelectedTrigger.  Deleting Root.Events must be refused
        // unless --force.
        let prj_with_triggers = create_prj_with_triggers();
        let err = delete_component(&prj_with_triggers, "Root.Events", true, false).unwrap_err();
        assert!(
            matches!(err, EditError::Invalid(_)),
            "expected Invalid, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("SelectedTrigger"),
            "error should mention SelectedTrigger"
        );
        // With --force it succeeds (even though the file is now broken).
        let out = delete_component(&prj_with_triggers, "Root.Events", true, true).unwrap();
        parses(&out);
        assert!(!out.contains("Root.Events"));
    }

    fn create_prj_with_triggers() -> String {
        // Build PRJ variant where Update and Sub.Tick have SelectedTriggers pointing
        // at Root.Events.On 100Hz.
        let mut out = set_call_rate(PRJ, "Root.Engine.Update", "100").unwrap();
        out = set_call_rate(&out, "Root.Engine.Sub.Tick", "100").unwrap();
        out
    }

    #[test]
    fn delete_preserves_surrounding_whitespace_tidiness() {
        // After deletion the file should still parse and should not have a blank line
        // where the component was (the whole line, break included, is consumed).
        let out = delete_component(PRJ, "Root.Engine.Plain", false, false).unwrap();
        parses(&out);
        assert!(
            !out.contains("\n\n"),
            "blank line left behind after delete:\n{out}"
        );
        assert_eq!(
            out.lines().count(),
            PRJ.lines().count() - 1,
            "exactly the deleted element's line should be gone"
        );
    }

    #[test]
    fn delete_consumes_crlf_line() {
        // Real .m1prj files are CRLF; the deleted line's \r\n must go too.
        let crlf = PRJ.replace('\n', "\r\n");
        let out = delete_component(&crlf, "Root.Engine.Plain", false, false).unwrap();
        parses(&out);
        assert!(
            !out.contains("\r\n\r\n"),
            "blank CRLF line left behind:\n{out}"
        );
        assert!(!out.contains("\n\n"), "mixed blank line left behind");
        assert!(
            !out.contains(r#"Name="Root.Engine.Plain""#),
            "component must be gone"
        );
    }

    // ---- #23 rename_component -----------------------------------------------

    #[test]
    fn rename_component_updates_name() {
        let (out, _warns) = rename_component(PRJ, "Root.Engine", "Motor").unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.Motor""#));
        assert!(!out.contains(r#"Name="Root.Engine""#));
        // Descendants also renamed.
        assert!(out.contains(r#"Name="Root.Motor.Speed""#));
        assert!(out.contains(r#"Name="Root.Motor.Plain""#));
    }

    #[test]
    fn rename_component_updates_selected_trigger() {
        // Set up a trigger pointing at Root.Events.On 100Hz.
        let prj = set_call_rate(PRJ, "Root.Engine.Update", "100").unwrap();
        // Now rename the Events group to "Clocks".
        let (out, _warns) = rename_component(&prj, "Root.Events", "Clocks").unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.Clocks""#));
        // The SelectedTrigger must be updated to the new name.
        // (Root.Engine.Update has 2 dots → climb 2 → Parent.Parent.Clocks.On 100Hz)
        assert!(
            out.contains("Clocks.On 100Hz"),
            "SelectedTrigger must reference the renamed group"
        );
        assert!(
            !out.contains("Events.On 100Hz"),
            "old SelectedTrigger must be gone"
        );
    }

    #[test]
    fn rename_component_rejects_duplicate() {
        assert!(matches!(
            rename_component(PRJ, "Root.Engine", "Events"),
            Err(EditError::Duplicate(_))
        ));
    }

    #[test]
    fn rename_component_rejects_missing() {
        assert!(matches!(
            rename_component(PRJ, "Root.Ghost", "X"),
            Err(EditError::NoSuchComponent(_))
        ));
    }

    #[test]
    fn rename_component_warns_about_script_files() {
        // Root.Engine.Update is a MethodUser → should produce a .m1scr warning.
        let (_, warns) = rename_component(PRJ, "Root.Engine", "Motor").unwrap();
        // Root.Motor.Update and Root.Motor.Sub.Tick both move.
        assert!(
            warns.iter().any(|w| w.contains("Motor.Update.m1scr")),
            "expected a warning for Motor.Update.m1scr, got: {warns:?}"
        );
    }

    // ---- #24 validate -------------------------------------------------------

    #[test]
    fn validate_clean_project() {
        let findings = validate(PRJ).unwrap();
        // PRJ has no SelectedTriggers, no duplicates → no findings.
        assert!(
            findings.is_empty(),
            "clean project should have no findings, got: {findings:?}"
        );
    }

    #[test]
    fn validate_with_valid_trigger() {
        // Add a trigger and validate — should be clean.
        let prj = set_call_rate(PRJ, "Root.Engine.Update", "100").unwrap();
        let findings = validate(&prj).unwrap();
        assert!(
            findings.is_empty(),
            "valid trigger should pass validate, got: {findings:?}"
        );
    }

    #[test]
    fn validate_detects_bad_trigger() {
        // Manually build a project with a SelectedTrigger pointing at a non-existent clock.
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
<Component Classname="BuiltIn.MethodUser" Name="Root.Script">
 <Props SelectedTrigger="Parent.Events.On 999Hz"/>
</Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let findings = validate(prj).unwrap();
        assert!(
            !findings.is_empty(),
            "bad SelectedTrigger should produce a finding"
        );
        assert!(findings.iter().any(|f| f.path == "Root.Script"));
    }

    #[test]
    fn validate_detects_duplicate_sibling_names() {
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
<Component Classname="BuiltIn.Channel" Name="Root.Speed"/>
<Component Classname="BuiltIn.Channel" Name="Root.Speed"/>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let findings = validate(prj).unwrap();
        assert!(
            findings.iter().any(|f| f.message.contains("duplicate")),
            "duplicate sibling should produce a finding, got: {findings:?}"
        );
    }

    #[test]
    fn validate_bad_xml_returns_error() {
        assert!(matches!(validate("<not valid xml"), Err(EditError::Xml(_))));
    }

    // ---- #25 list_components ------------------------------------------------

    #[test]
    fn list_components_returns_all_in_order() {
        let entries = list_components(PRJ).unwrap();
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        // Document order: Root, Root.Engine, Root.Engine.Speed, …
        assert_eq!(paths[0], "Root");
        assert_eq!(paths[1], "Root.Engine");
        assert_eq!(paths[2], "Root.Engine.Speed");
    }

    #[test]
    fn list_components_reads_props() {
        let entries = list_components(PRJ).unwrap();
        let speed = entries
            .iter()
            .find(|e| e.path == "Root.Engine.Speed")
            .unwrap();
        assert_eq!(speed.ty.as_deref(), Some("f32"));
        assert_eq!(speed.security.as_deref(), Some("Tune"));
        assert!(speed.unit.is_none());
    }

    #[test]
    fn list_components_reads_unit() {
        let prj = set_unit(PRJ, "Root.Engine.Speed", "rpm").unwrap();
        let entries = list_components(&prj).unwrap();
        let speed = entries
            .iter()
            .find(|e| e.path == "Root.Engine.Speed")
            .unwrap();
        assert_eq!(speed.unit.as_deref(), Some("rpm"));
    }

    #[test]
    fn list_components_depth_is_dot_count() {
        let entries = list_components(PRJ).unwrap();
        let root = entries.iter().find(|e| e.path == "Root").unwrap();
        let engine = entries.iter().find(|e| e.path == "Root.Engine").unwrap();
        let speed = entries
            .iter()
            .find(|e| e.path == "Root.Engine.Speed")
            .unwrap();
        assert_eq!(root.depth, 0);
        assert_eq!(engine.depth, 1);
        assert_eq!(speed.depth, 2);
    }

    // ---- resolve_trigger / build_trigger helpers ----------------------------

    #[test]
    fn resolve_trigger_two_parents() {
        // Root.Engine.Update (2 dots) + Parent.Parent.Events.On 100Hz → Root.Events.On 100Hz
        let abs = resolve_trigger("Root.Engine.Update", "Parent.Parent.Events.On 100Hz");
        assert_eq!(abs.as_deref(), Some("Root.Events.On 100Hz"));
    }

    #[test]
    fn resolve_trigger_three_parents() {
        let abs = resolve_trigger(
            "Root.Engine.Sub.Tick",
            "Parent.Parent.Parent.Events.On 100Hz",
        );
        assert_eq!(abs.as_deref(), Some("Root.Events.On 100Hz"));
    }

    #[test]
    fn build_trigger_round_trips() {
        let owner = "Root.Engine.Update";
        let clock = "Root.Events.On 100Hz";
        let trigger = build_trigger(owner, clock);
        let abs = resolve_trigger(owner, &trigger).unwrap();
        assert_eq!(abs, clock);
    }
}
