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
mod edits;
mod query;
mod validate;
mod xml;

pub use edits::{
    create_channel, create_group, delete_component, rename_component, set_call_rate, set_security,
    set_type, set_unit,
};
pub use query::{ComponentEntry, available_rates, list_components, resolve_trigger};
pub use validate::{Finding, FindingLevel, validate};

#[cfg(test)]
pub(crate) use edits::build_trigger;

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
