//! Low-level XML location and splicing helpers shared by every edit:
//! byte-accurate `roxmltree` lookups (`locate`/`parse_and_find`), the splice
//! primitives (`splice`/`xml_escape`/`indent_at`) and the `<Props>` attribute
//! machinery (`set_props_attr`/`ensure_props`/`props_*`/`default_unit_*`).

use crate::EditError;

/// Parse `xml` into a `roxmltree::Document`, mapping a parse failure to
/// [`EditError::Xml`] with the same message every call site used.
pub(crate) fn parse_xml(xml: &str) -> Result<roxmltree::Document<'_>, EditError> {
    roxmltree::Document::parse(xml).map_err(|e| EditError::Xml(e.to_string()))
}

/// True if `n` is a **real** `<Component>` — one that carries a `Classname`
/// attribute, i.e. an entry in a `<ComponentStream>`'s flat `<List>`.
///
/// The `<Organisation>` view tree also nests `<Component>` elements, but those
/// are short-name, view-only structural nodes with **no** `Classname`; they must
/// be excluded from every structural query/edit/validation pass. This single
/// definition is the source of truth for that distinction — see the
/// `<Organisation>` navigation note further down this file for why the two
/// representations coexist.
pub(crate) fn is_real_component(n: &roxmltree::Node) -> bool {
    n.has_tag_name("Component") && n.has_attribute("Classname")
}

/// Find the `<Component>` whose `Name` is `component` in an already-parsed `doc`,
/// erroring with [`EditError::NoSuchComponent`] if absent. The caller owns `doc`
/// so the returned `Node` (which borrows it) stays valid — see the LIFETIME NOTE
/// above for why the `Document` is not created/returned here.
pub(crate) fn parse_and_find<'a, 'd>(
    doc: &'d roxmltree::Document<'a>,
    component: &str,
) -> Result<roxmltree::Node<'d, 'a>, EditError> {
    doc.descendants()
        .find(|n| is_real_component(n) && n.attribute("Name") == Some(component))
        .ok_or_else(|| EditError::NoSuchComponent(component.to_string()))
}

/// A located component: its name, class, byte range of the whole `<Component>`
/// element, and the byte range of its `<Props>` child if present.
pub(crate) struct Located {
    pub(crate) classname: String,
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) props_range: Option<std::ops::Range<usize>>,
}

/// Find a component by its fully-qualified `Name`, returning its layout.
pub(crate) fn locate(xml: &str, name: &str) -> Result<Located, EditError> {
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

/// The security groups (`<SecurityRole Name="…">`) the project declares in its
/// `<SecurityMgr><SecurityRoles>` block, in document order.
///
/// Returns `Ok(None)` when the project has **no** `<SecurityMgr>` element at all —
/// minimal/hand-written projects and the manual's "Automatic" tag-derived
/// security mode (where no explicit role list exists). Callers treat that as
/// "fall back to the standard default groups" rather than rejecting everything.
/// `Ok(Some(roles))` (possibly empty) means the project declares its roles
/// explicitly and those are the only groups M1-Build will bind.
pub(crate) fn declared_security_roles(xml: &str) -> Result<Option<Vec<String>>, EditError> {
    let doc = parse_xml(xml)?;
    let Some(mgr) = doc.descendants().find(|n| n.has_tag_name("SecurityMgr")) else {
        return Ok(None);
    };
    let roles = mgr
        .descendants()
        .filter(|n| n.has_tag_name("SecurityRole"))
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect();
    Ok(Some(roles))
}

/// True if a component with this exact `Name` exists (only considers real components
/// that carry a `Classname` attribute, excluding `<Organisation>` view-only nodes).
pub(crate) fn exists(xml: &str, name: &str) -> Result<bool, EditError> {
    let doc = parse_xml(xml)?;
    Ok(doc
        .descendants()
        .any(|n| is_real_component(&n) && n.attribute("Name") == Some(name)))
}

/// The leading whitespace (indentation) of the line containing byte `pos`.
pub(crate) fn indent_at(xml: &str, pos: usize) -> &str {
    let line_start = xml[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rest = &xml[line_start..];
    let end = rest
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The parent path of a dotted name (`Root.A.B` -> `Root.A`), or `None` for a
/// single segment.
pub(crate) fn parent_of(name: &str) -> Option<&str> {
    name.rfind('.').map(|i| &name[..i])
}

/// Set/replace an attribute on a component's `<Props>`, creating `<Props>` if absent.
pub(crate) fn set_props_attr(
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

/// Remove an attribute (` attr="value"`, leading space included) from a
/// component's `<Props>` opening tag, located via the parser so the splice can
/// never land inside another attribute's quoted value. No-op (returns the input
/// unchanged) if there is no `<Props>` or the attribute is absent.
pub(crate) fn remove_props_attr(
    xml: &str,
    component: &str,
    attr: &str,
) -> Result<String, EditError> {
    let range = {
        let doc = parse_xml(xml)?;
        let comp = parse_and_find(&doc, component)?;
        comp.children()
            .find(|c| c.has_tag_name("Props"))
            .and_then(|p| p.attribute_node(attr))
            .map(|a| a.range())
    };
    let Some(r) = range else {
        return Ok(xml.to_string());
    };
    // `Attribute::range()` spans `attr="value"`; also consume the single space
    // that separates it from the previous attribute / tag name.
    let start = if xml[..r.start].ends_with(' ') {
        r.start - 1
    } else {
        r.start
    };
    Ok(splice(xml, start..r.end, ""))
}

/// The `<Entry Value="…">` user tags on this component's
/// `<Props><List.UserTags>`, in document order (empty if there is no such block).
pub(crate) fn user_tags(xml: &str, component: &str) -> Result<Vec<String>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|p| p.children().find(|c| c.has_tag_name("List.UserTags")))
        .map(|tags| {
            tags.children()
                .filter(|e| e.has_tag_name("Entry"))
                .filter_map(|e| e.attribute("Value"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

/// The byte range of this component's `<Props><List.UserTags>` element, if present.
pub(crate) fn user_tags_range(
    xml: &str,
    component: &str,
) -> Result<Option<std::ops::Range<usize>>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|p| p.children().find(|c| c.has_tag_name("List.UserTags")))
        .map(|n| n.range()))
}

/// The byte range of this component's `<Comment>` child element, if present.
pub(crate) fn comment_range(
    xml: &str,
    component: &str,
) -> Result<Option<std::ops::Range<usize>>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Comment"))
        .map(|n| n.range()))
}

/// Ensure the component has a `<Props>` child; if it is `<Component …/>`
/// (self-closing) rewrite it to `<Component …><Props/></Component>`.
pub(crate) fn ensure_props(xml: &str, component: &str) -> Result<String, EditError> {
    let loc = locate(xml, component)?;
    if loc.props_range.is_some() {
        return Ok(xml.to_string());
    }
    let elem = &xml[loc.range.clone()];
    let indent = indent_at(xml, loc.range.start).to_string();
    if props_self_closing(elem) {
        let new = reopen_self_closing(
            elem,
            &format!("\n{indent} <Props/>"),
            &format!("\n{indent}</Component>"),
        );
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

pub(crate) fn props_self_closing(props_text: &str) -> bool {
    props_text.trim_end().ends_with("/>")
}

/// Reopen a self-closing element and nest content inside it.
///
/// `elem_text` is the source text of an element the caller has already confirmed
/// is self-closing (`<Tag …/>`, via [`props_self_closing`]). This strips the
/// trailing `/>`, turns it into an open tag, and appends `child_block` followed
/// by `close_marker` — e.g. `reopen_self_closing("<Props/>", "\n  <Locale/>",
/// "\n </Props>")` yields `"<Props>\n  <Locale/>\n </Props>"`.
///
/// Both fragments are supplied verbatim by the caller because indentation and the
/// matching close tag differ per call site; this helper owns only the
/// `trim_end().strip_suffix("/>")` reopen step that every self-closing splice
/// repeated. It is the shared half of [`insert_child_into_element`].
pub(crate) fn reopen_self_closing(
    elem_text: &str,
    child_block: &str,
    close_marker: &str,
) -> String {
    let open = elem_text
        .trim_end()
        .strip_suffix("/>")
        .expect("caller must check props_self_closing first");
    format!("{open}>{child_block}{close_marker}")
}

/// Rewrite a whole element's text to nest one extra child just inside it,
/// returning the new element text (the caller splices it back over the original
/// range). Handles both shapes:
///
/// - **self-closing** (`<Tag …/>`): reopened via [`reopen_self_closing`], then
///   `child_block` and `\n{indent}{close_tag}` appended.
/// - **already has children** (`<Tag …> … {close_tag}`): the last `close_tag` is
///   found, the text before it trimmed of trailing whitespace, and
///   `child_block` + `\n{indent}{close_tag}` spliced in just before it.
///
/// `child_block` must include its own leading newline/indent (the corpus layout
/// puts every child on its own line). `indent` is the element's own indentation;
/// the close tag is re-laid at that column. Errors with [`EditError::Invalid`]
/// if a non-self-closing element is missing its `close_tag` (malformed input).
///
/// This is the whole-element-rewrite primitive shared by the `<Props>` /
/// `<List.UserTags>` insert paths. Splices that need byte-exact offsets into the
/// *original* document (set_comment / ensure_props / org_insert_child has-children
/// arms) deliberately do **not** route through here.
pub(crate) fn insert_child_into_element(
    elem_text: &str,
    child_block: &str,
    indent: &str,
    close_tag: &str,
) -> Result<String, EditError> {
    let close_marker = format!("\n{indent}{close_tag}");
    if props_self_closing(elem_text) {
        Ok(reopen_self_closing(elem_text, child_block, &close_marker))
    } else {
        let close = elem_text
            .rfind(close_tag)
            .ok_or_else(|| EditError::Invalid(format!("malformed {close_tag}")))?;
        Ok(format!(
            "{}{child_block}{close_marker}",
            elem_text[..close].trim_end()
        ))
    }
}

/// The byte range (in `xml`) of the value of `attr` on the target component's
/// `<Props>` opening tag, located via the XML parser. `roxmltree`'s
/// `range_value()` is the exact span between the quotes, so the replace can never
/// land inside another attribute's quoted value (#5).
pub(crate) fn props_attr_value_range(
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

/// The byte range of `attr`'s value on this component's `<Default>` element
/// (`<Props><Locale><Default attr="…"/>`) — the Display-section fields Unit,
/// Format, DPS, Min, Max — located via the XML parser so an `attr="…"` in a
/// comment or a non-`Default` element is ignored rather than mutated (#7).
pub(crate) fn default_attr_value_range(
    xml: &str,
    component: &str,
    attr: &str,
) -> Result<Option<std::ops::Range<usize>>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|props| {
            props
                .descendants()
                .find(|d| d.has_tag_name("Default") && d.has_attribute(attr))
        })
        .and_then(|d| d.attribute_node(attr))
        .map(|a| a.range_value()))
}

/// The byte offset just after the `<Default` tag name of this component's
/// existing `<Props><Locale><Default …>` element that has **no** `attr`
/// attribute yet — the point to splice ` attr="…"` into. `None` if there is no
/// such `<Default>` (so the caller falls back to creating the whole `<Locale>`).
pub(crate) fn default_attr_insert_point(
    xml: &str,
    component: &str,
    attr: &str,
) -> Result<Option<usize>, EditError> {
    let doc = parse_xml(xml)?;
    let comp = parse_and_find(&doc, component)?;
    Ok(comp
        .children()
        .find(|c| c.has_tag_name("Props"))
        .and_then(|props| {
            props
                .descendants()
                .find(|d| d.has_tag_name("Default") && !d.has_attribute(attr))
        })
        // node.range() spans the whole element; insert right after `<Default`.
        .map(|d| d.range().start + "<Default".len()))
}

// ---- <Organisation> view-tree navigation ------------------------------------
//
// Each `<ComponentStream>` pairs a flat `<List>` of real components (every one
// carrying a `Classname`) with a nested `<Organisation>` tree that mirrors the
// hierarchy using **short** names (one `<Component Name="leaf">` per level, no
// `Classname`). M1-Build binds each object's Properties through `<Organisation>`,
// so a structural edit (create/delete/rename) that touches only `<List>` leaves
// the two out of sync and M1-Build then refuses to load the project
// ("Unable to find Properties for object 'Root.X'"). These helpers let the edits
// keep `<Organisation>` consistent. Projects without any `<Organisation>` (e.g.
// minimal/hand-written ones) simply yield `None` and the edits become no-ops on
// the view tree.

/// Walk an `<Organisation>` element by the short-name `segments` of a dotted path
/// (`Root.CAN.EPOS` -> `["Root","CAN","EPOS"]`), returning the matching view node.
fn walk_org<'a, 'd>(
    org: roxmltree::Node<'d, 'a>,
    segments: &[&str],
) -> Option<roxmltree::Node<'d, 'a>> {
    let mut cur = org;
    for seg in segments {
        cur = cur
            .children()
            .find(|c| c.has_tag_name("Component") && c.attribute("Name") == Some(*seg))?;
    }
    Some(cur)
}

/// A located view node: `(whole-element range, Name-attribute-value range)`.
type OrgNodeRanges = (std::ops::Range<usize>, std::ops::Range<usize>);

/// Locate the `<Organisation>` view node for component `path`, returning
/// `(whole-element range, Name-attribute-value range)`. Searches every
/// `<Organisation>` tree (a project may have one per `<ComponentStream>`).
/// `Ok(None)` when there is no matching view node (no `<Organisation>` at all,
/// or the path is absent from the view tree).
pub(crate) fn org_locate(xml: &str, path: &str) -> Result<Option<OrgNodeRanges>, EditError> {
    let doc = parse_xml(xml)?;
    let segments: Vec<&str> = path.split('.').collect();
    for org in doc.descendants().filter(|n| n.has_tag_name("Organisation")) {
        if let Some(node) = walk_org(org, &segments) {
            let name_value = node
                .attribute_node("Name")
                .map(|a| a.range_value())
                .ok_or_else(|| EditError::Invalid("Organisation node lacks Name".into()))?;
            return Ok(Some((node.range(), name_value)));
        }
    }
    Ok(None)
}

/// Insert a `<Component Name="leaf"/>` view node as the last child of
/// `parent_path`'s `<Organisation>` node, returning the rewritten XML. `Ok(None)`
/// when there is no `<Organisation>` node for `parent_path` to extend (the edit
/// then leaves the view tree untouched).
pub(crate) fn org_insert_child(
    xml: &str,
    parent_path: &str,
    leaf: &str,
) -> Result<Option<String>, EditError> {
    let (parent_range, _) = match org_locate(xml, parent_path)? {
        Some(loc) => loc,
        None => return Ok(None),
    };
    let parent_text = &xml[parent_range.clone()];
    let parent_indent = indent_at(xml, parent_range.start).to_string();
    // `<Organisation>` nests one space deeper per level throughout the corpus.
    let child_indent = format!("{parent_indent} ");
    let child = format!("\n{child_indent}<Component Name=\"{}\"/>", xml_escape(leaf));

    if props_self_closing(parent_text) {
        // Childless `<Component Name="X"/>` -> open it and nest the new child.
        let new = reopen_self_closing(
            parent_text,
            &child,
            &format!("\n{parent_indent}</Component>"),
        );
        Ok(Some(splice(xml, parent_range, &new)))
    } else {
        // Has children: splice the new child in just before the parent's own
        // closing `</Component>` (the LAST one in the element's text).
        let close_rel = parent_text
            .rfind("</Component>")
            .ok_or_else(|| EditError::Invalid("malformed Organisation node".into()))?;
        let abs_close = parent_range.start + close_rel;
        let line_start = xml[..abs_close].rfind('\n').unwrap_or(abs_close);
        Ok(Some(splice(xml, line_start..line_start, &child)))
    }
}

/// Extend `start` backwards over the preceding indentation and its line break
/// (LF or CRLF) so deleting `[extended_start..end)` removes the element's whole
/// line and leaves no blank line behind. Shared by `<List>` and `<Organisation>`
/// deletions.
pub(crate) fn line_extended_start(xml: &str, start: usize) -> usize {
    let before = &xml[..start];
    let ws_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prefix_is_ws = xml[ws_start..start].chars().all(|c| c == ' ' || c == '\t');
    if prefix_is_ws && ws_start > 0 {
        if xml[..ws_start - 1].ends_with('\r') {
            ws_start - 2
        } else {
            ws_start - 1
        }
    } else if prefix_is_ws {
        ws_start
    } else {
        start
    }
}

pub(crate) fn splice(s: &str, range: std::ops::Range<usize>, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len() - (range.end - range.start) + replacement.len());
    out.push_str(&s[..range.start]);
    out.push_str(replacement);
    out.push_str(&s[range.end..]);
    out
}

pub(crate) fn xml_escape(s: &str) -> String {
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
    <Component Classname="BuiltIn.Channel" Name="Root.Speed"><Props Type="f32"/></Component>
   </List>
   <Organisation>
    <Component Name="Root">
     <Component Name="Speed"/>
    </Component>
   </Organisation>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

    #[test]
    fn is_real_component_accepts_classname_carrying_list_nodes() {
        let doc = parse_xml(PRJ).expect("valid xml");
        // The flat <List> nodes carry a Classname — they are the real components.
        let real: Vec<&str> = doc
            .descendants()
            .filter(is_real_component)
            .filter_map(|n| n.attribute("Name"))
            .collect();
        assert_eq!(real, vec!["Root", "Root.Speed"]);
    }

    #[test]
    fn is_real_component_rejects_organisation_view_nodes() {
        let doc = parse_xml(PRJ).expect("valid xml");
        // Every <Organisation> <Component> lacks a Classname (view-only structural
        // node) and must be excluded by the predicate.
        let org = doc
            .descendants()
            .find(|n| n.has_tag_name("Organisation"))
            .expect("fixture has an Organisation tree");
        for view_node in org.descendants().filter(|n| n.has_tag_name("Component")) {
            assert!(
                !is_real_component(&view_node),
                "Organisation view node {:?} must not count as a real component",
                view_node.attribute("Name")
            );
        }
    }

    #[test]
    fn is_real_component_rejects_non_component_elements() {
        let doc = parse_xml(PRJ).expect("valid xml");
        let props = doc
            .descendants()
            .find(|n| n.has_tag_name("Props"))
            .expect("fixture has a Props element");
        assert!(!is_real_component(&props));
    }

    #[test]
    fn reopen_self_closing_strips_slash_and_nests_block() {
        assert_eq!(
            reopen_self_closing("<Props/>", "\n  <Locale/>", "\n </Props>"),
            "<Props>\n  <Locale/>\n </Props>"
        );
        // Trailing whitespace after `/>` is tolerated (matches the corpus, where
        // the element text can carry a trailing newline before the next sibling).
        assert_eq!(
            reopen_self_closing("<Props Type=\"f32\"/>\n", "\n  X", "\n</Props>"),
            "<Props Type=\"f32\">\n  X\n</Props>"
        );
    }

    #[test]
    fn insert_child_into_element_reopens_self_closing() {
        // Self-closing <Props/> at one space of indent, nesting a <List.UserTags>
        // block one space deeper — the shape add_tag/set_default_attr produce.
        let block = "\n  <List.UserTags>\n   <Entry Value=\"V\"/>\n  </List.UserTags>";
        let got = insert_child_into_element("<Props/>", block, " ", "</Props>").unwrap();
        assert_eq!(
            got,
            "<Props>\n  <List.UserTags>\n   <Entry Value=\"V\"/>\n  </List.UserTags>\n </Props>"
        );
    }

    #[test]
    fn insert_child_into_element_splices_before_close_when_has_children() {
        // Element already has children: the new block lands just before the last
        // close tag, trailing whitespace before that close tag trimmed away.
        let elem = "<Props Type=\"f32\">\n  <Locale/>\n </Props>";
        let block = "\n  <List.UserTags>\n   <Entry Value=\"V\"/>\n  </List.UserTags>";
        let got = insert_child_into_element(elem, block, " ", "</Props>").unwrap();
        assert_eq!(
            got,
            "<Props Type=\"f32\">\n  <Locale/>\
             \n  <List.UserTags>\n   <Entry Value=\"V\"/>\n  </List.UserTags>\n </Props>"
        );
    }

    #[test]
    fn insert_child_into_element_rejects_missing_close_tag() {
        // Not self-closing and no close tag present -> malformed.
        let err = insert_child_into_element("<Props Type=\"f32\">", "\n X", "", "</Props>");
        assert!(matches!(err, Err(EditError::Invalid(_))));
    }
}
