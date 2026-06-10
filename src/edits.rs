//! The mutating verbs: `create_*`, `delete_component`, `rename_component`,
//! the `set_*` property edits, and their value/name validators. Every edit is
//! a pure `&str -> Result<String, EditError>` built on [`crate::xml`]'s
//! locate-and-splice primitives.

use crate::query::{available_rates, resolve_trigger};
use crate::xml::*;
use crate::{EditError, SECURITY_LEVELS, STORAGE_TYPES};

/// A backing-script file the CLI should rename on disk after [`rename_component`].
/// `old`/`new` are paths relative to the project's `Scripts/` directory (exactly
/// the `.m1scr` files M1-Build renames when you rename a script in its UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRename {
    pub old: String,
    pub new: String,
}

/// Shared insert primitive: place a new `<Component>` as the last *direct* child
/// of `name`'s parent group (at the parent's indentation), then mirror it into the
/// `<Organisation>` view tree. `attrs` are extra attributes emitted right after
/// `Classname` (e.g. ` Filename="…"`); `body(indent)` is the element's inner
/// content — `None` makes the element self-closing, `Some(s)` wraps it as
/// `<Component …>{s}\n{indent}</Component>` (so `s` must carry its own leading
/// `\n{indent} …`). This is exactly how M1-Build itself serialises an insert.
fn insert_component(
    xml: &str,
    name: &str,
    classname: &str,
    attrs: &str,
    body: impl FnOnce(&str) -> Option<String>,
) -> Result<String, EditError> {
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

    let element = match body(&indent) {
        None => format!(
            "\n{indent}<Component Classname=\"{classname}\"{attrs} Name=\"{}\"/>",
            xml_escape(name)
        ),
        Some(inner) => format!(
            "\n{indent}<Component Classname=\"{classname}\"{attrs} Name=\"{}\">{inner}\n{indent}</Component>",
            xml_escape(name)
        ),
    };

    let mut out = String::with_capacity(xml.len() + element.len());
    out.push_str(&xml[..anchor_end]);
    out.push_str(&element);
    out.push_str(&xml[anchor_end..]);

    // Mirror the new component into the `<Organisation>` view tree so M1-Build
    // shows it and can bind its Properties (no-op if the project has no view tree).
    let leaf = name.rsplit('.').next().unwrap_or(name);
    if let Some(synced) = org_insert_child(&out, parent, leaf)? {
        out = synced;
    }
    Ok(out)
}

/// The body M1-Build writes for a value component (Channel/Parameter): a `<Props>`
/// when any of type/unit/security is set, otherwise the empty `<Comment/>`
/// placeholder it emits for a default insert.
fn value_body(
    indent: &str,
    ty: Option<&str>,
    unit: Option<&str>,
    security: Option<&str>,
) -> Option<String> {
    let props = build_props(ty, unit, security);
    if props.is_empty() {
        Some(format!("\n{indent} <Comment/>"))
    } else {
        Some(format!("\n{indent} {props}"))
    }
}

/// The conventional backing-script path for a script component: the fully-qualified
/// name with the leading `Root.` dropped, plus `.m1scr`
/// (`Root.Control.Drive State.Update` → `Control.Drive State.Update.m1scr`). This is
/// the `Filename` M1-Build stores and the file it creates/renames under `Scripts/`.
pub fn script_relpath(name: &str) -> String {
    let stem = name.strip_prefix("Root.").unwrap_or(name);
    format!("{stem}.m1scr")
}

/// Create a `BuiltIn.Channel` under its (existing) parent group. `ty`/`unit`/
/// `security` are optional; with none set the element is the bare `<Comment/>`
/// form M1-Build writes by default.
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
    insert_component(xml, name, "BuiltIn.Channel", "", |indent| {
        value_body(indent, ty, unit, security)
    })
}

/// Create a `BuiltIn.Parameter` (a value tunable in M1 Tune) under its parent.
/// Same shape as [`create_channel`].
pub fn create_parameter(
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
    insert_component(xml, name, "BuiltIn.Parameter", "", |indent| {
        value_body(indent, ty, unit, security)
    })
}

/// Create a `BuiltIn.GroupCompound` under its (existing) parent group.
pub fn create_group(xml: &str, name: &str) -> Result<String, EditError> {
    validate_name_segment(name)?;
    insert_component(xml, name, "BuiltIn.GroupCompound", "", |indent| {
        Some(format!("\n{indent} <Comment/>"))
    })
}

/// Create a `BuiltIn.FuncUser` scheduled function under its parent. M1-Build
/// stores the backing script as `Filename` and creates the empty `.m1scr` on disk
/// — the CLI creates that file (see [`script_relpath`]); the element itself is
/// self-closing.
pub fn create_scheduled_function(xml: &str, name: &str) -> Result<String, EditError> {
    validate_name_segment(name)?;
    let attrs = format!(" Filename=\"{}\"", xml_escape(&script_relpath(name)));
    insert_component(xml, name, "BuiltIn.FuncUser", &attrs, |_| None)
}

/// Create a `BuiltIn.FuncUserParam` (parametric) function under its parent. Like a
/// scheduled function it carries a `Filename`, plus the empty `<Signature>` block
/// M1-Build writes for a new parametric function.
pub fn create_function(xml: &str, name: &str) -> Result<String, EditError> {
    validate_name_segment(name)?;
    let attrs = format!(" Filename=\"{}\"", xml_escape(&script_relpath(name)));
    insert_component(xml, name, "BuiltIn.FuncUserParam", &attrs, |indent| {
        // Byte-for-byte the empty signature M1-Build emits; the CDATA lines sit at
        // column 0 (no indent), matching M1-Build's serialiser.
        Some(format!(
            "\n{i} <Signature Name=\"\">\n{i}  <Description>\n<![CDATA[]]>\n{i}  </Description>\n{i}  <DescriptionFull>\n<![CDATA[]]>\n{i}  </DescriptionFull>\n{i} </Signature>",
            i = indent
        ))
    })
}

/// Render the `<Props>` child for a value component from the optional type/unit/security.
pub(crate) fn build_props(ty: Option<&str>, unit: Option<&str>, security: Option<&str>) -> String {
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

    // Remove the matching `<Organisation>` view node. Its descendants are nested
    // inside it, so the single removal takes the whole subtree with it — and the
    // line is consumed so no blank line is left behind (no-op without a view tree).
    if let Some((range, _)) = org_locate(&result, name)? {
        let start = line_extended_start(&result, range.start);
        result = splice(&result, start..range.end, "");
    }
    Ok(result)
}

/// Rename a component, updating its `Name` attribute and every `SelectedTrigger`
/// in the file whose resolved absolute path matches the old name or any of its
/// descendants.
///
/// `new_segment` must be a single identifier (no dots). Also updates the
/// `<Organisation>` view node and the `Filename` of every renamed script
/// component, and returns the rewritten XML plus the backing `.m1scr` files the
/// caller should rename on disk (old → new), matching M1-Build's UI rename.
pub fn rename_component(
    xml: &str,
    old_name: &str,
    new_segment: &str,
) -> Result<(String, Vec<ScriptRename>), EditError> {
    // `--new-name` is the new *leaf* segment only. A dotted value here is the
    // classic misuse that silently produced a doubled path
    // (`Root.CAN.` + `Root.CAN.Foo`); reject it explicitly rather than corrupt
    // the project. (`validate_name_segment` is lenient — it only checks the last
    // dot-segment — because `create_group` legitimately takes a full path.)
    if new_segment.contains('.') {
        return Err(EditError::Invalid(format!(
            "--new-name must be a single segment with no dots, got `{new_segment}`; \
             pass just the new leaf name, e.g. `Motor`"
        )));
    }
    validate_name_segment(new_segment)?;

    // Compute what the new full name will be.
    let new_name = match parent_of(old_name) {
        Some(parent) => format!("{parent}.{new_segment}"),
        None => new_segment.to_string(),
    };

    if new_name == old_name {
        return Ok((xml.to_string(), Vec::<ScriptRename>::new()));
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

    // Backing-script files M1-Build renames on disk: each renamed FuncUser/
    // FuncUserParam (or legacy MethodUser) maps its old → new `.m1scr` path.
    let script_renames: Vec<ScriptRename> = to_rename
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
        .map(|(old, new)| ScriptRename {
            old: script_relpath(old),
            new: script_relpath(new),
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

    // Pass 3: update the `Filename` of every renamed script component to its new
    // conventional `.m1scr` path. Without this the renamed FuncUser/FuncUserParam
    // still points at its old backing file (a dangling reference); M1-Build
    // rewrites it (and renames the file — the CLI does that via `script_renames`).
    {
        let doc = parse_xml(&result)?;
        let mut fixes: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for n in doc
            .descendants()
            .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        {
            let Some(owner_new) = n.attribute("Name") else {
                continue;
            };
            // Only components we just renamed carry a `new` name; skip the rest.
            if !to_rename.iter().any(|(_, new)| new == owner_new) {
                continue;
            }
            if let Some(fa) = n.attribute_node("Filename") {
                fixes.push((fa.range_value(), script_relpath(owner_new)));
            }
        }
        fixes.sort_by_key(|r| std::cmp::Reverse(r.0.start));
        for (range, new_val) in fixes {
            result = splice(&result, range, &xml_escape(&new_val));
        }
    }

    // Pass 4: rename the matching `<Organisation>` view node. The view tree uses
    // short names with descendants nested inside, so ONLY this node's segment
    // changes — its children keep their (unchanged) short names. Navigate by the
    // OLD path: earlier passes rewrote `<List>` Names/triggers/Filenames but left
    // the view tree alone, so it still carries the old segment here. (No-op
    // without a view tree.) Missing this is what made M1-Build fail to load the
    // project ("Unable to find Properties for object 'Root.X'").
    if let Some((_, name_value)) = org_locate(&result, old_name)? {
        result = splice(&result, name_value, &xml_escape(new_segment));
    }

    Ok((result, script_renames))
}

// ---- validate ---------------------------------------------------------------

/// Build a group-relative `SelectedTrigger` value for `owner` pointing at
/// `abs_clock` (an absolute component path).
pub(crate) fn build_trigger(owner: &str, abs_clock: &str) -> String {
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
pub(crate) fn validate_name_segment(name: &str) -> Result<(), EditError> {
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
    set_default_attr(xml, component, "Unit", unit)
}

/// Set (or replace) a component's display **Format** (`<Default Format>`, e.g.
/// `Hex`, `Default`) — the M1-Build *Display → Format* row.
pub fn set_format(xml: &str, component: &str, format: &str) -> Result<String, EditError> {
    if format.trim().is_empty() {
        return Err(EditError::Invalid("format must not be empty".into()));
    }
    set_default_attr(xml, component, "Format", format)
}

/// Set (or replace) a component's decimal places (`<Default DPS>`) — the
/// *Display → DPS* row. `dps` is a non-negative integer.
pub fn set_dps(xml: &str, component: &str, dps: u32) -> Result<String, EditError> {
    set_default_attr(xml, component, "DPS", &dps.to_string())
}

/// Set a component's display **Min/Max** (`<Default Min=… Max=…>`, in M1-Build's
/// `%.17e` form) — the *Display → Minimum/Maximum* rows. These are the display
/// clamp, distinct from the *Validation* `ValMin`/`ValMax` (see [`set_validation`]).
pub fn set_display_range(
    xml: &str,
    component: &str,
    min: f64,
    max: f64,
) -> Result<String, EditError> {
    if min > max {
        return Err(EditError::Invalid(format!(
            "display min ({min}) must not exceed max ({max})"
        )));
    }
    let out = set_default_attr(xml, component, "Min", &format_motec_float(min))?;
    set_default_attr(&out, component, "Max", &format_motec_float(max))
}

/// Set (or replace) an attribute on a value component's `<Props><Locale><Default>`
/// element — the Display-section fields (Unit/Format/DPS/Min/Max). Creates the
/// `<Props>`/`<Locale>`/`<Default>` chain as needed; reuses an existing
/// `<Default>` (so a sibling DPS/Format/unit is never orphaned by a duplicate
/// `<Locale>`).
fn set_default_attr(
    xml: &str,
    component: &str,
    attr: &str,
    value: &str,
) -> Result<String, EditError> {
    let xml = ensure_props(xml, component)?;

    // Replace the value of the real `<Default attr>`, located via the XML parser
    // so a match inside a comment or an unrelated element is never mutated (#7).
    if let Some(v_range) = default_attr_value_range(&xml, component, attr)? {
        return Ok(splice(&xml, v_range, &xml_escape(value)));
    }

    // A `<Default>` may already exist carrying other attributes but not this one
    // (the corpus is full of `<Default Unit="%" DPS="2"/>`). Add the attribute to
    // *that* element rather than appending a second `<Locale>`, which would
    // duplicate the element and orphan its siblings.
    if let Some(insert_at) = default_attr_insert_point(&xml, component, attr)? {
        let frag = format!(" {attr}=\"{}\"", xml_escape(value));
        return Ok(splice(&xml, insert_at..insert_at, &frag));
    }

    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    let pindent = indent_at(&xml, props_range.start).to_string();
    let props_text = &xml[props_range.clone()];

    // M1-Build's serialiser nests one space per level with every element on its
    // own line (same layout as `add_tag`'s `<List.UserTags>`).
    let block = format!(
        "\n{pindent} <Locale>\n{pindent}  <Default {attr}=\"{}\"/>\n{pindent} </Locale>",
        xml_escape(value)
    );
    let new_props = if props_self_closing(props_text) {
        let open = props_text
            .trim_end()
            .strip_suffix("/>")
            .expect("checked by props_self_closing above");
        format!("{open}>{block}\n{pindent}</Props>")
    } else {
        // `<Props …> … </Props>` — insert the Locale just before `</Props>`.
        let close_idx = props_text
            .rfind("</Props>")
            .ok_or_else(|| EditError::Invalid("malformed <Props>".into()))?;
        format!(
            "{}{block}\n{pindent}{}",
            props_text[..close_idx].trim_end(),
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

/// Set (or replace) a component's physical **Quantity** (`<Props Qty="…">`) — the
/// dimension M1-Build shows in the *Value → Quantity* row (e.g. `ratio`, `rad/s`,
/// `Hz`). This is distinct from the display *unit* (`set_unit` → `<Default Unit>`);
/// the quantity is the underlying physical kind, the unit a way of displaying it.
pub fn set_quantity(xml: &str, component: &str, qty: &str) -> Result<String, EditError> {
    if qty.trim().is_empty() {
        return Err(EditError::Invalid("quantity must not be empty".into()));
    }
    set_props_attr(xml, component, "Qty", qty)
}

/// Set the *Validation* of a value component (the M1-Build *Validation* section).
/// `kind` is `MinMax` (then `min`/`max` are required) or `None`/`none` (clears the
/// `Validation`/`ValMin`/`ValMax` attributes). Bounds are written in the
/// `%.17e` form M1-Build uses (see `format_motec_float`).
pub fn set_validation(
    xml: &str,
    component: &str,
    kind: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<String, EditError> {
    match kind {
        "None" | "none" => {
            let mut out = remove_props_attr(xml, component, "Validation")?;
            out = remove_props_attr(&out, component, "ValMin")?;
            out = remove_props_attr(&out, component, "ValMax")?;
            Ok(out)
        }
        "MinMax" => {
            let min = min.ok_or_else(|| {
                EditError::Invalid("MinMax validation needs --min and --max".into())
            })?;
            let max = max.ok_or_else(|| {
                EditError::Invalid("MinMax validation needs --min and --max".into())
            })?;
            if min > max {
                return Err(EditError::Invalid(format!(
                    "validation min ({min}) must not exceed max ({max})"
                )));
            }
            // `set_props_attr` prepends a freshly-added attribute right after
            // `<Props`, so insert in reverse (ValMax, ValMin, Validation) to leave
            // the triple reading `Validation ValMin ValMax` left-to-right as
            // M1-Build writes it. (A pre-existing attribute is replaced in place,
            // so re-running keeps the order.)
            let mut out = set_props_attr(xml, component, "ValMax", &format_motec_float(max))?;
            out = set_props_attr(&out, component, "ValMin", &format_motec_float(min))?;
            out = set_props_attr(&out, component, "Validation", "MinMax")?;
            Ok(out)
        }
        other => Err(EditError::Invalid(format!(
            "unknown validation type `{other}`; valid: MinMax, None"
        ))),
    }
}

/// Format a bound the way M1-Build serialises `ValMin`/`ValMax`: C `%.17e` —
/// a single mantissa digit, 17 fractional digits, lowercase `e`, an explicit
/// sign and a ≥2-digit exponent (`1.00000000000000000e+00`). M1 re-reads the
/// value as the same `f64` regardless, but matching the form keeps diffs clean.
pub(crate) fn format_motec_float(v: f64) -> String {
    let s = format!("{v:.17e}"); // e.g. "1.00000000000000000e0", "5.00000000000000000e-1"
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let (sign, digits) = match exp.strip_prefix('-') {
        Some(d) => ('-', d),
        None => ('+', exp.strip_prefix('+').unwrap_or(exp)),
    };
    format!("{mantissa}e{sign}{digits:0>2}")
}

/// Add a user **Tag** to a component (`<Props><List.UserTags><Entry Value="tag"/>`).
/// This is the *Tags* row in M1-Build's Properties; a component missing a tag its
/// class requires is what M1-Build's *Validate Project* reports as
/// "Mandatory tag not selected". Creates `<Props>` and/or `<List.UserTags>` as
/// needed; idempotent (a tag already present leaves the file unchanged).
pub fn add_tag(xml: &str, component: &str, tag: &str) -> Result<String, EditError> {
    if tag.trim().is_empty() {
        return Err(EditError::Invalid("tag must not be empty".into()));
    }
    let xml = ensure_props(xml, component)?;
    if user_tags(&xml, component)?.iter().any(|t| t == tag) {
        return Ok(xml); // already present
    }
    let entry = format!("<Entry Value=\"{}\"/>", xml_escape(tag));

    // A <List.UserTags> already exists: append the entry inside it, on its own
    // line one space deeper than the element — M1-Build's serialiser puts every
    // <Entry> on its own line (see any List.UserTags in a real project).
    if let Some(range) = user_tags_range(&xml, component)? {
        let indent = indent_at(&xml, range.start).to_string();
        let text = &xml[range.clone()];
        let new = if props_self_closing(text) {
            let open = text
                .trim_end()
                .strip_suffix("/>")
                .expect("checked by props_self_closing");
            format!("{open}>\n{indent} {entry}\n{indent}</List.UserTags>")
        } else {
            let close = text
                .rfind("</List.UserTags>")
                .ok_or_else(|| EditError::Invalid("malformed <List.UserTags>".into()))?;
            format!(
                "{}\n{indent} {entry}\n{indent}{}",
                text[..close].trim_end(),
                &text[close..]
            )
        };
        return Ok(splice(&xml, range, &new));
    }

    // No <List.UserTags> yet: add one inside <Props>, nested one space per level
    // at the <Props> indentation (M1-Build's layout).
    let loc = locate(&xml, component)?;
    let props_range = loc.props_range.expect("ensure_props guarantees <Props>");
    let pindent = indent_at(&xml, props_range.start).to_string();
    let props_text = &xml[props_range.clone()];
    let block =
        format!("\n{pindent} <List.UserTags>\n{pindent}  {entry}\n{pindent} </List.UserTags>");
    let new_props = if props_self_closing(props_text) {
        let open = props_text
            .trim_end()
            .strip_suffix("/>")
            .expect("checked by props_self_closing");
        format!("{open}>{block}\n{pindent}</Props>")
    } else {
        let close = props_text
            .rfind("</Props>")
            .ok_or_else(|| EditError::Invalid("malformed <Props>".into()))?;
        format!(
            "{}{block}\n{pindent}{}",
            props_text[..close].trim_end(),
            &props_text[close..]
        )
    };
    Ok(splice(&xml, props_range, &new_props))
}

/// Remove a user tag from a component. Errors if the component does not carry that
/// tag. When the last tag is removed, the now-empty `<List.UserTags>` element is
/// dropped entirely (its whole line, if it was on its own line).
pub fn remove_tag(xml: &str, component: &str, tag: &str) -> Result<String, EditError> {
    let tags = user_tags(xml, component)?;
    if !tags.iter().any(|t| t == tag) {
        return Err(EditError::Invalid(format!(
            "`{component}` has no user tag `{tag}`"
        )));
    }
    let range = user_tags_range(xml, component)?
        .ok_or_else(|| EditError::Invalid("no <List.UserTags> to edit".into()))?;
    let remaining: Vec<String> = tags.into_iter().filter(|t| t != tag).collect();
    if remaining.is_empty() {
        // Drop the whole element, consuming its leading indentation+newline if it
        // sits on its own line (inline → just the element).
        let start = line_extended_start(xml, range.start);
        return Ok(splice(xml, start..range.end, ""));
    }
    // Rewrite the surviving entries one per line, one space deeper than the
    // element (M1-Build's serialiser layout — same as `add_tag`).
    let indent = indent_at(xml, range.start).to_string();
    let entries: String = remaining
        .iter()
        .map(|t| format!("\n{indent} <Entry Value=\"{}\"/>", xml_escape(t)))
        .collect();
    Ok(splice(
        xml,
        range,
        &format!("<List.UserTags>{entries}\n{indent}</List.UserTags>"),
    ))
}

pub(crate) fn validate_security(s: &str) -> Result<(), EditError> {
    if SECURITY_LEVELS.contains(&s) {
        Ok(())
    } else {
        Err(EditError::Invalid(format!(
            "unknown security level `{s}`; valid: {}",
            SECURITY_LEVELS.join(", ")
        )))
    }
}

pub(crate) fn validate_type(t: &str) -> Result<(), EditError> {
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
