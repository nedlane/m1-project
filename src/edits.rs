//! The mutating verbs: `create_*`, `delete_component`, `rename_component`,
//! the `set_*` property edits, and their value/name validators. Every edit is
//! a pure `&str -> Result<String, EditError>` built on [`crate::xml`]'s
//! locate-and-splice primitives.

use crate::query::{available_rates, resolve_trigger};
use crate::xml::*;
use crate::{EditError, SECURITY_LEVELS, STORAGE_TYPES};

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
