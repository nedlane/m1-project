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
//! - [`create_constant`] — add a `BuiltIn.Constant` with its literal `Value`.
//! - [`create_table`] — add a `BuiltIn.Table` with 1–3 axes.
//! - [`create_group`] — add a `BuiltIn.GroupCompound` under an existing group.
//! - [`delete_component`] — remove a component element (and optionally its subtree).
//! - [`rename_component`] — rename a component and update all `SelectedTrigger` references.
//! - [`set_security`] — set/replace a component's `<Props Security="…">`.
//! - [`set_unit`] — set/replace a component's display unit (`<Locale><Default Unit>`).
//! - [`set_type`] — set/replace a component's storage `Type`.
//! - [`set_quantity`] — set/replace a component's physical quantity (`<Props Qty>`).
//! - [`set_validation`] — set/clear a value component's `Validation`/`ValMin`/`ValMax`.
//! - [`set_format`]/[`set_dps`]/[`set_display_range`] — the Display-section
//!   `<Default>` fields (Format, DPS, Min/Max).
//! - [`add_tag`]/[`remove_tag`] — manage a component's `<List.UserTags>` (the *Tags* row).
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
    ScriptRename, TableAxis, add_tag, create_channel, create_constant, create_function,
    create_group, create_parameter, create_reference, create_scheduled_function, create_table,
    delete_component, remove_tag, rename_component, script_relpath, set_call_rate, set_comment,
    set_display_range, set_dps, set_format, set_quantity, set_security, set_type, set_unit,
    set_validation,
};
pub use query::{
    ComponentEntry, ScriptComponent, available_rates, list_components, resolve_trigger,
    script_components,
};
pub use validate::{Finding, FindingLevel, validate};

#[cfg(test)]
pub(crate) use edits::{build_trigger, format_motec_float, validate_type};

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
    fn create_channel_rejects_empty_unit() {
        // An empty/whitespace unit would write `<Default Unit=""/>` — invalid;
        // mirror set_unit, which already rejects it.
        assert!(matches!(
            create_channel(PRJ, "Root.Engine.X", Some("f32"), Some(""), None),
            Err(EditError::Invalid(_))
        ));
        assert!(matches!(
            create_channel(PRJ, "Root.Engine.X", Some("f32"), Some("   "), None),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn create_parameter_rejects_empty_unit() {
        assert!(matches!(
            create_parameter(PRJ, "Root.Engine.X", Some("f32"), Some(""), None),
            Err(EditError::Invalid(_))
        ));
        assert!(matches!(
            create_parameter(PRJ, "Root.Engine.X", Some("f32"), Some("\t"), None),
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
    fn set_unit_rejects_empty_string() {
        assert!(
            matches!(
                set_unit(PRJ, "Root.Engine.Speed", ""),
                Err(EditError::Invalid(_))
            ),
            "empty unit must return EditError::Invalid"
        );
        assert!(
            matches!(
                set_unit(PRJ, "Root.Engine.Speed", "   "),
                Err(EditError::Invalid(_))
            ),
            "whitespace-only unit must return EditError::Invalid"
        );
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
    fn set_call_rate_rejects_func_user_param() {
        // BuiltIn.FuncUserParam is a parametric (called) function — it has no
        // SelectedTrigger slot and must not be schedulable.  The guard must
        // distinguish "FuncUserParam" from "FuncUser" rather than relying on a
        // substring match that lets FuncUserParam pass.
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
<Component Classname="BuiltIn.FuncUserParam" Filename="Calc.m1scr" Name="Root.Calc"/>
<Component Classname="BuiltIn.GroupCompound" Name="Root.Events"/>
<Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let result = set_call_rate(prj, "Root.Calc", "100");
        assert!(
            matches!(result, Err(EditError::Invalid(_))),
            "FuncUserParam must be rejected by set_call_rate, got: {result:?}"
        );
        // The error message must distinguish parametric from scheduled functions.
        if let Err(EditError::Invalid(msg)) = result {
            assert!(
                msg.contains("parametric") || msg.contains("FuncUserParam"),
                "error should mention parametric or FuncUserParam, got: {msg}"
            );
        }
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

    // ---- #39 create_constant / create_table ----------------------------------

    #[test]
    fn create_constant_writes_props_value() {
        let out = create_constant(PRJ, "Root.Engine.Bus", "CAN Bus 1").unwrap();
        parses(&out);
        assert!(out.contains(r#"Classname="BuiltIn.Constant" Name="Root.Engine.Bus""#));
        assert!(out.contains(r#"<Props Value="CAN Bus 1"/>"#));
        // The insert introduces no new validate() findings.
        assert_eq!(validate(&out).unwrap().len(), validate(PRJ).unwrap().len());
    }

    #[test]
    fn create_constant_rejects_duplicate_and_empty_value() {
        assert!(matches!(
            create_constant(PRJ, "Root.Engine.Speed", "x"),
            Err(EditError::Duplicate(_))
        ));
        assert!(matches!(
            create_constant(PRJ, "Root.Engine.Bus", "  "),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn create_table_relativizes_absolute_axis_source() {
        let axes = [TableAxis {
            source: "Root.Engine.Speed".into(),
            sites: Some(11),
        }];
        let out = create_table(PRJ, "Root.Engine.Pedal Map", &axes, Some("Tune")).unwrap();
        parses(&out);
        assert!(out.contains(r#"Classname="BuiltIn.Table" Name="Root.Engine.Pedal Map""#));
        assert!(out.contains(r#"<Props Security="Tune" NumAxes="1">"#));
        // `Root.Engine.Speed` from `Root.Engine.Pedal Map` is one climb + leaf,
        // the same group-relative form SelectedTrigger uses.
        assert!(out.contains(r#"<X Source="Parent.Speed" MaxSites="11"/>"#));
        assert_eq!(validate(&out).unwrap().len(), validate(PRJ).unwrap().len());
    }

    #[test]
    fn create_table_three_axes_passthrough_relative_sources() {
        let axes = [
            TableAxis {
                source: "Root.Engine.Speed".into(),
                sites: Some(11),
            },
            TableAxis {
                source: "Parent.Plain".into(),
                sites: Some(5),
            },
            TableAxis {
                source: "Parent.Parent.Events.On 100Hz".into(),
                sites: None,
            },
        ];
        let out = create_table(PRJ, "Root.Engine.Map", &axes, None).unwrap();
        parses(&out);
        assert!(out.contains(r#"NumAxes="3""#));
        assert!(out.contains(r#"<Y Source="Parent.Plain" MaxSites="5"/>"#));
        assert!(out.contains(r#"<Z Source="Parent.Parent.Events.On 100Hz"/>"#));
    }

    #[test]
    fn create_table_rejects_bad_axes() {
        // No axes at all.
        assert!(matches!(
            create_table(PRJ, "Root.Engine.M2", &[], None),
            Err(EditError::Invalid(_))
        ));
        // An absolute source that doesn't exist in the project.
        let missing = [TableAxis {
            source: "Root.Engine.Nope".into(),
            sites: None,
        }];
        assert!(matches!(
            create_table(PRJ, "Root.Engine.M3", &missing, None),
            Err(EditError::Invalid(_))
        ));
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
        // Root.Engine.Update is a MethodUser → should report a .m1scr rename.
        let (_, renames) = rename_component(PRJ, "Root.Engine", "Motor").unwrap();
        // Root.Motor.Update and Root.Motor.Sub.Tick both move; old→new is reported.
        assert!(
            renames
                .iter()
                .any(|r| r.new == "Motor.Update.m1scr" && r.old == "Engine.Update.m1scr"),
            "expected Engine.Update.m1scr → Motor.Update.m1scr, got: {renames:?}"
        );
    }

    // ---- #24 validate -------------------------------------------------------

    #[test]
    fn validate_clean_project() {
        let findings = validate(PRJ).unwrap();
        // PRJ's only structural gap is the deliberately-bare `Root.Engine.Plain`
        // channel, which Check 6 flags for a missing Security group (= M1-Build
        // Error 1601). No OTHER findings (no bad triggers, no duplicates).
        assert!(
            findings
                .iter()
                .all(|f| f.path == "Root.Engine.Plain" && f.message.contains("security")),
            "only the bare Plain channel should be flagged: {findings:?}"
        );
    }

    #[test]
    fn validate_with_valid_trigger() {
        // Add a trigger and validate — the trigger must pass; the only remaining
        // finding is the bare Plain channel's missing Security (Check 6).
        let prj = set_call_rate(PRJ, "Root.Engine.Update", "100").unwrap();
        let findings = validate(&prj).unwrap();
        assert!(
            findings.iter().all(|f| f.path == "Root.Engine.Plain"),
            "a valid trigger must not be flagged: {findings:?}"
        );
    }

    #[test]
    fn validate_flags_channel_without_security() {
        // Root.Engine.Plain is a bare channel with no <Props Security>.
        let findings = validate(PRJ).unwrap();
        assert!(
            findings.iter().any(|f| f.path == "Root.Engine.Plain"
                && f.level == FindingLevel::Error
                && f.message.contains("security")),
            "bare channel must be flagged for missing security: {findings:?}"
        );
        // A channel WITH security (Root.Engine.Speed) is not flagged.
        assert!(
            !findings
                .iter()
                .any(|f| f.path == "Root.Engine.Speed" && f.message.contains("security")),
            "a channel with security must not be flagged: {findings:?}"
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

    // ---- <Organisation> view-tree sync -------------------------------------
    //
    // A real `.m1prj` carries the hierarchy twice: the flat `<List>` of real
    // components AND a nested `<Organisation>` view tree (short names, no
    // Classname) that M1-Build binds Properties through. Structural edits must
    // keep both in sync or M1-Build fails to load the project. This fixture
    // mirrors PRJ's components in an `<Organisation>` so the sync is exercised.

    const PRJ_ORG: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="T">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Engine"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Engine.Speed">
     <Props Type="f32" Security="Tune"/>
    </Component>
    <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Update"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Events"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
   </List>
   <Organisation>
    <Component Name="Root">
     <Component Name="Engine">
      <Component Name="Speed"/>
      <Component Name="Update"/>
     </Component>
     <Component Name="Events">
      <Component Name="On 100Hz"/>
     </Component>
    </Component>
   </Organisation>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

    #[test]
    fn create_channel_syncs_organisation() {
        // Give it a security group so the result is a complete, clean component
        // (Check 6 flags security-less channels — see create_channel_bare_…).
        let out = create_channel(
            PRJ_ORG,
            "Root.Engine.Torque",
            Some("f32"),
            None,
            Some("Tune"),
        )
        .unwrap();
        parses(&out);
        // Added to the List…
        assert!(out.contains(r#"Classname="BuiltIn.Channel" Name="Root.Engine.Torque""#));
        // …AND as a short-name node inside the <Organisation> Engine group.
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(
            org.contains(r#"<Component Name="Torque"/>"#),
            "new channel must appear in the Organisation view:\n{org}"
        );
        // The project stays internally consistent.
        assert!(
            validate(&out).unwrap().is_empty(),
            "List/Organisation must stay in sync: {:?}",
            validate(&out).unwrap()
        );
    }

    #[test]
    fn create_group_syncs_organisation() {
        let out = create_group(PRJ_ORG, "Root.Engine.SubSystem").unwrap();
        parses(&out);
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(org.contains(r#"<Component Name="SubSystem"/>"#));
        assert!(validate(&out).unwrap().is_empty());
    }

    #[test]
    fn delete_syncs_organisation() {
        let out = delete_component(PRJ_ORG, "Root.Engine.Update", false, false).unwrap();
        parses(&out);
        // Gone from BOTH List and Organisation (so no dangling Properties ref).
        assert!(!out.contains(r#"Name="Root.Engine.Update""#));
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(
            !org.contains(r#"<Component Name="Update"/>"#),
            "deleted component must be removed from the Organisation view too:\n{org}"
        );
        assert!(validate(&out).unwrap().is_empty());
        assert!(!out.contains("\n\n"), "no blank line left behind:\n{out}");
    }

    #[test]
    fn delete_group_recursive_syncs_organisation() {
        let out = delete_component(PRJ_ORG, "Root.Engine", true, false).unwrap();
        parses(&out);
        assert!(!out.contains("Root.Engine"));
        let org = &out[out.find("<Organisation>").unwrap()..];
        // The whole Engine subtree (incl. Speed/Update) is gone from the view.
        assert!(!org.contains(r#"Name="Engine""#));
        assert!(!org.contains(r#"Name="Speed""#));
        assert!(validate(&out).unwrap().is_empty());
    }

    #[test]
    fn rename_syncs_organisation() {
        let (out, _warns) = rename_component(PRJ_ORG, "Root.Engine", "Motor").unwrap();
        parses(&out);
        // List renamed (self + descendants).
        assert!(out.contains(r#"Name="Root.Motor""#));
        assert!(out.contains(r#"Name="Root.Motor.Speed""#));
        assert!(!out.contains(r#"Name="Root.Engine""#));
        // Organisation: the one node's short name changes; children keep theirs.
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(
            org.contains(r#"<Component Name="Motor">"#),
            "view node must be renamed:\n{org}"
        );
        assert!(!org.contains(r#"<Component Name="Engine">"#));
        assert!(
            org.contains(r#"<Component Name="Speed"/>"#),
            "child short name unchanged"
        );
        // No dangling Properties reference — M1-Build would load this.
        assert!(
            validate(&out).unwrap().is_empty(),
            "rename must leave List/Organisation consistent: {:?}",
            validate(&out).unwrap()
        );
    }

    #[test]
    fn rename_rejects_dotted_new_name() {
        // The misuse that silently doubled the path (Root.CAN.Root.CAN.Foo).
        let err = rename_component(PRJ_ORG, "Root.Engine", "Root.Motor").unwrap_err();
        assert!(
            matches!(err, EditError::Invalid(_)),
            "dotted --new-name must be rejected, got {err:?}"
        );
    }

    #[test]
    fn ops_without_organisation_still_work() {
        // PRJ has no <Organisation>; edits must still succeed (view sync no-ops).
        let out = create_channel(PRJ, "Root.Engine.New", None, None, None).unwrap();
        parses(&out);
        assert!(out.contains(r#"Name="Root.Engine.New""#));
        let (out2, _) = rename_component(PRJ, "Root.Engine", "Motor").unwrap();
        parses(&out2);
        assert!(out2.contains(r#"Name="Root.Motor""#));
    }

    #[test]
    fn validate_detects_dangling_organisation_node() {
        // A view node ("Ghost") with no matching real component — the exact shape
        // a List-only rename/delete used to leave behind.
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream>
<List>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
</List>
<Organisation>
<Component Name="Root"><Component Name="Ghost"/></Component>
</Organisation>
</ComponentStream></Project></MoTeCM1BuildSession>"#;
        let findings = validate(prj).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.level == FindingLevel::Error && f.path == "Root.Ghost"),
            "dangling Organisation node must be an error, got: {findings:?}"
        );
    }

    // ---- new Built-in create verbs (match M1-Build's UI serialisation) ------

    #[test]
    fn create_channel_bare_emits_comment_like_ui() {
        // A default channel insert in M1-Build is `<…><Comment/></…>`, not a
        // self-closing tag.
        let out = create_channel(PRJ_ORG, "Root.Engine.Bare", None, None, None).unwrap();
        parses(&out);
        let at = out.find(r#"Name="Root.Engine.Bare""#).unwrap();
        assert!(
            out[at..at + 200].contains("<Comment/>"),
            "bare channel should carry <Comment/>"
        );
        // Like an M1-Build UI insert, a bare channel has no security yet, so it is
        // flagged (Error 1601) until `set-security` — and nothing else is wrong.
        let findings = validate(&out).unwrap();
        assert!(
            findings
                .iter()
                .all(|f| f.path == "Root.Engine.Bare" && f.message.contains("security")),
            "only the missing-security finding expected: {findings:?}"
        );
    }

    #[test]
    fn create_parameter_syncs_and_serialises() {
        let out = create_parameter(PRJ_ORG, "Root.Engine.Gain", None, None, Some("Tune")).unwrap();
        parses(&out);
        assert!(
            out.contains(r#"<Component Classname="BuiltIn.Parameter" Name="Root.Engine.Gain">"#)
        );
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(org.contains(r#"<Component Name="Gain"/>"#));
        assert!(validate(&out).unwrap().is_empty());
    }

    #[test]
    fn create_scheduled_function_has_filename_and_is_self_closing() {
        let out = create_scheduled_function(PRJ_ORG, "Root.Engine.Tick").unwrap();
        parses(&out);
        assert!(out.contains(
            r#"<Component Classname="BuiltIn.FuncUser" Filename="Engine.Tick.m1scr" Name="Root.Engine.Tick"/>"#
        ));
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(org.contains(r#"<Component Name="Tick"/>"#));
        // A freshly-inserted scheduled function has no event yet — validate flags
        // it exactly as M1-Build does ("no event selected"). Assigning a rate
        // clears the finding.
        assert!(
            validate(&out)
                .unwrap()
                .iter()
                .any(|f| f.path == "Root.Engine.Tick" && f.message.contains("no event")),
            "new scheduled function should be flagged as eventless: {:?}",
            validate(&out).unwrap()
        );
        let wired = set_call_rate(&out, "Root.Engine.Tick", "100").unwrap();
        assert!(
            validate(&wired).unwrap().is_empty(),
            "scheduled function with a rate should validate clean: {:?}",
            validate(&wired).unwrap()
        );
    }

    // ---- property setters discovered from M1-Build's Properties tab ----------

    #[test]
    fn set_quantity_sets_and_replaces() {
        let out = set_quantity(PRJ, "Root.Engine.Speed", "rad/s").unwrap();
        parses(&out);
        assert!(out.contains(r#"Qty="rad/s""#));
        // Type is untouched.
        assert!(out.contains(r#"Type="f32""#));
        let out2 = set_quantity(&out, "Root.Engine.Speed", "Hz").unwrap();
        assert!(out2.contains(r#"Qty="Hz""#) && !out2.contains(r#"Qty="rad/s""#));
    }

    #[test]
    fn set_quantity_on_props_less_component() {
        // Root.Engine.Plain is self-closing with no <Props>.
        let out = set_quantity(PRJ, "Root.Engine.Plain", "ratio").unwrap();
        parses(&out);
        assert!(out.contains(r#"Qty="ratio""#));
    }

    #[test]
    fn format_motec_float_matches_m1build() {
        assert_eq!(format_motec_float(0.0), "0.00000000000000000e+00");
        assert_eq!(format_motec_float(1.0), "1.00000000000000000e+00");
        assert_eq!(format_motec_float(100.0), "1.00000000000000000e+02");
        assert_eq!(format_motec_float(0.5), "5.00000000000000000e-01");
    }

    #[test]
    fn set_validation_minmax_writes_bounds() {
        let out = set_validation(PRJ, "Root.Engine.Speed", "MinMax", Some(0.0), Some(1.0)).unwrap();
        parses(&out);
        assert!(out.contains(r#"Validation="MinMax""#));
        assert!(out.contains(r#"ValMin="0.00000000000000000e+00""#));
        assert!(out.contains(r#"ValMax="1.00000000000000000e+00""#));
        // Clearing removes all three attributes again.
        let cleared = set_validation(&out, "Root.Engine.Speed", "None", None, None).unwrap();
        parses(&cleared);
        assert!(!cleared.contains("Validation="));
        assert!(!cleared.contains("ValMin="));
        assert!(!cleared.contains("ValMax="));
        // The unrelated Type attribute survives the clear.
        assert!(cleared.contains(r#"Type="f32""#));
    }

    #[test]
    fn set_validation_minmax_requires_bounds() {
        assert!(matches!(
            set_validation(PRJ, "Root.Engine.Speed", "MinMax", None, None),
            Err(EditError::Invalid(_))
        ));
        assert!(matches!(
            set_validation(PRJ, "Root.Engine.Speed", "MinMax", Some(2.0), Some(1.0)),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn set_format_dps_and_display_range_on_default() {
        // Start from a channel with an existing <Default Unit> so we exercise the
        // "add attr to existing <Default>" path and never duplicate <Locale>.
        let base = set_unit(PRJ, "Root.Engine.Speed", "rpm").unwrap();
        let out = set_format(&base, "Root.Engine.Speed", "Hex").unwrap();
        let out = set_dps(&out, "Root.Engine.Speed", 2).unwrap();
        let out = set_display_range(&out, "Root.Engine.Speed", -360.0, 360.0).unwrap();
        parses(&out);
        assert_eq!(out.matches("<Locale>").count(), 1, "single <Locale>");
        assert_eq!(out.matches("<Default").count(), 1, "single <Default>");
        assert!(out.contains(r#"Unit="rpm""#));
        assert!(out.contains(r#"Format="Hex""#));
        assert!(out.contains(r#"DPS="2""#));
        assert!(out.contains(r#"Min="-3.60000000000000000e+02""#));
        assert!(out.contains(r#"Max="3.60000000000000000e+02""#));
        // Replacing a value keeps a single attribute.
        let out2 = set_dps(&out, "Root.Engine.Speed", 4).unwrap();
        assert!(out2.contains(r#"DPS="4""#) && !out2.contains(r#"DPS="2""#));
        assert_eq!(out2.matches("DPS=").count(), 1);
    }

    #[test]
    fn set_display_range_creates_default_chain_from_scratch() {
        // Root.Engine.Plain has no <Props>; the whole chain must be built.
        let out = set_display_range(PRJ, "Root.Engine.Plain", 0.0, 1.0).unwrap();
        parses(&out);
        assert!(
            out.contains(
                r#"<Default Min="0.00000000000000000e+00" Max="1.00000000000000000e+00"/>"#
            ) || (out.contains(r#"Min="0.00000000000000000e+00""#)
                && out.contains(r#"Max="1.00000000000000000e+00""#))
        );
        assert!(out.contains("<Locale>"));
    }

    #[test]
    fn set_display_range_rejects_inverted() {
        assert!(matches!(
            set_display_range(PRJ, "Root.Engine.Speed", 5.0, 1.0),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn add_tag_creates_props_and_list() {
        // Root.Engine.Plain has no <Props>; add_tag must build the whole chain.
        let out = add_tag(PRJ, "Root.Engine.Plain", "Vehicle").unwrap();
        parses(&out);
        // M1-Build's layout: one space deeper per level, every <Entry> on its
        // own line.
        assert!(
            out.contains(
                "\n      <List.UserTags>\n       <Entry Value=\"Vehicle\"/>\n      </List.UserTags>"
            ),
            "M1-Build multi-line List.UserTags layout:\n{out}"
        );
        // Idempotent: re-adding the same tag is a no-op.
        let again = add_tag(&out, "Root.Engine.Plain", "Vehicle").unwrap();
        assert_eq!(again, out);
        // A second tag appends a second <Entry> in the same <List.UserTags>.
        let two = add_tag(&out, "Root.Engine.Plain", "Tune").unwrap();
        parses(&two);
        assert_eq!(two.matches("<List.UserTags>").count(), 1);
        assert!(two.contains(r#"<Entry Value="Vehicle"/>"#));
        assert!(two.contains(r#"<Entry Value="Tune"/>"#));
    }

    #[test]
    fn add_tag_into_existing_props() {
        // Root.Engine.Speed already has `<Props Type="f32" Security="Tune"/>`.
        let out = add_tag(PRJ, "Root.Engine.Speed", "Tune").unwrap();
        parses(&out);
        assert!(out.contains(r#"Type="f32""#) && out.contains(r#"Security="Tune""#));
        assert!(
            out.contains(
                "\n      <List.UserTags>\n       <Entry Value=\"Tune\"/>\n      </List.UserTags>"
            ),
            "M1-Build multi-line List.UserTags layout:\n{out}"
        );
    }

    #[test]
    fn remove_tag_removes_entry_and_empties_list() {
        let one = add_tag(PRJ, "Root.Engine.Plain", "Vehicle").unwrap();
        let two = add_tag(&one, "Root.Engine.Plain", "Tune").unwrap();
        // Remove one of two: the list survives with the other.
        let back = remove_tag(&two, "Root.Engine.Plain", "Tune").unwrap();
        parses(&back);
        assert!(back.contains(r#"<Entry Value="Vehicle"/>"#));
        assert!(!back.contains(r#"<Entry Value="Tune"/>"#));
        // Remove the last: the <List.UserTags> element disappears entirely.
        let none = remove_tag(&back, "Root.Engine.Plain", "Vehicle").unwrap();
        parses(&none);
        assert!(!none.contains("<List.UserTags>"));
    }

    #[test]
    fn remove_tag_absent_errors() {
        assert!(matches!(
            remove_tag(PRJ, "Root.Engine.Speed", "Ghost"),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn validate_flags_eventless_scheduled_function() {
        let prj = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="T"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
<Component Classname="BuiltIn.FuncUser" Filename="Sched.m1scr" Name="Root.Sched"/>
<Component Classname="BuiltIn.FuncUserParam" Filename="Fn.m1scr" Name="Root.Fn"/>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#;
        let findings = validate(prj).unwrap();
        // The scheduled function (FuncUser) is flagged…
        assert!(
            findings.iter().any(|f| f.path == "Root.Sched"
                && f.level == FindingLevel::Error
                && f.message.contains("no event")),
            "eventless FuncUser must be an error: {findings:?}"
        );
        // …but the parametric function (FuncUserParam) is NOT (it is called, not scheduled).
        assert!(
            !findings.iter().any(|f| f.path == "Root.Fn"),
            "FuncUserParam must not be flagged for a missing event: {findings:?}"
        );
    }

    #[test]
    fn create_function_has_filename_and_signature() {
        let out = create_function(PRJ_ORG, "Root.Engine.Calc").unwrap();
        parses(&out);
        assert!(out.contains(
            r#"<Component Classname="BuiltIn.FuncUserParam" Filename="Engine.Calc.m1scr" Name="Root.Engine.Calc">"#
        ));
        assert!(out.contains(r#"<Signature Name="">"#));
        assert!(out.contains("<![CDATA[]]>"));
        let org = &out[out.find("<Organisation>").unwrap()..];
        assert!(org.contains(r#"<Component Name="Calc"/>"#));
        assert!(validate(&out).unwrap().is_empty());
    }

    #[test]
    fn script_relpath_strips_root_and_adds_ext() {
        assert_eq!(
            script_relpath("Root.Control.Drive State.Update"),
            "Control.Drive State.Update.m1scr"
        );
    }

    #[test]
    fn rename_script_updates_filename_and_reports_rename() {
        let prj = create_scheduled_function(PRJ_ORG, "Root.Engine.Tick").unwrap();
        let (out, renames) = rename_component(&prj, "Root.Engine.Tick", "Tock").unwrap();
        parses(&out);
        // The Filename follows the new name (no dangling reference).
        assert!(
            out.contains(r#"Filename="Engine.Tock.m1scr""#),
            "Filename must update:\n{out}"
        );
        assert!(!out.contains(r#"Filename="Engine.Tick.m1scr""#));
        // And the old→new file rename is reported for the CLI.
        assert!(
            renames
                .iter()
                .any(|r| r.old == "Engine.Tick.m1scr" && r.new == "Engine.Tock.m1scr"),
            "expected Tick→Tock pair, got {renames:?}"
        );
        // The scheduled function still needs an event (Check 5); give it one, then
        // the project is structurally clean.
        let wired = set_call_rate(&out, "Root.Engine.Tock", "100").unwrap();
        assert!(
            validate(&wired).unwrap().is_empty(),
            "renamed+wired scheduled function should validate clean: {:?}",
            validate(&wired).unwrap()
        );
    }

    // ---- validate_type: malformed enum-ref rejection (#58) ------------------

    #[test]
    fn validate_type_rejects_bare_double_colon() {
        assert!(
            matches!(validate_type("::"), Err(EditError::Invalid(_))),
            "\"::\" must be rejected as a malformed enum ref"
        );
    }

    #[test]
    fn validate_type_rejects_bare_dot() {
        assert!(
            matches!(validate_type("."), Err(EditError::Invalid(_))),
            "\".\" must be rejected as a malformed enum ref"
        );
    }

    #[test]
    fn validate_type_rejects_triple_dot() {
        assert!(
            matches!(validate_type("..."), Err(EditError::Invalid(_))),
            "\"...\" must be rejected as a malformed enum ref"
        );
    }

    #[test]
    fn validate_type_rejects_triple_colon() {
        assert!(
            matches!(validate_type(":::"), Err(EditError::Invalid(_))),
            "\":::\" must be rejected as a malformed enum ref"
        );
    }

    #[test]
    fn validate_type_accepts_valid_enum_refs() {
        // `::This.Foo` style: double-colon prefix followed by non-empty Namespace.Member
        assert!(
            validate_type("::This.Foo").is_ok(),
            "\"::This.Foo\" must be accepted"
        );
        // `MoTeC Types.Bar` style: dotted qualified name with non-empty segments
        assert!(
            validate_type("MoTeC Types.Bar").is_ok(),
            "\"MoTeC Types.Bar\" must be accepted"
        );
    }

    #[test]
    fn validate_type_accepts_all_primitives() {
        for &ty in STORAGE_TYPES {
            assert!(
                validate_type(ty).is_ok(),
                "primitive type \"{ty}\" must be accepted"
            );
        }
    }

    #[test]
    fn set_type_rejects_bare_double_colon() {
        assert!(
            matches!(
                set_type(PRJ, "Root.Engine.Plain", "::"),
                Err(EditError::Invalid(_))
            ),
            "set_type with \"::\" must return EditError::Invalid"
        );
    }
}
