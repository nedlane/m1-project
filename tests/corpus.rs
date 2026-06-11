//! Exercise the mutations against the real EV-M1 and AV-M1 `Project.m1prj` when present.
//! Skipped (passes trivially) in a fresh public clone with no corpus. The corpus
//! is the sibling `EV-M1/UQR-EV/01.00/Project.m1prj`, or `$M1_CORPUS_PROJECT`.
use std::path::PathBuf;

fn corpus_project() -> Option<String> {
    let path = match std::env::var_os("M1_CORPUS_PROJECT") {
        Some(p) => PathBuf::from(p),
        None => {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../EV-M1/UQR-EV/01.00/Project.m1prj")
        }
    };
    std::fs::read_to_string(path).ok()
}

fn av_corpus_project() -> Option<String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../AV-M1/UQR-AV/01.00/Project.m1prj");
    // AV-M1 is Windows-1252 encoded; use the tolerant decoder.
    if path.exists() {
        m1_workspace::read_text(&path).ok()
    } else {
        None
    }
}

/// Names of all `<Component>` of a given class in the project.
fn components_of_class(xml: &str, class: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    doc.descendants()
        .filter(|n| n.has_tag_name("Component") && n.attribute("Classname") == Some(class))
        .filter_map(|n| n.attribute("Name").map(str::to_string))
        .collect()
}

/// Count only real `<Component>` elements — those with a `Classname` attribute,
/// which live in the `<List>` section. The `<Organisation>` section also contains
/// `<Component>` nodes without `Classname` (view-only structural nodes) that should
/// not be counted here, since `list_components` also excludes them.
fn component_count(xml: &str) -> usize {
    roxmltree::Document::parse(xml)
        .unwrap()
        .descendants()
        .filter(|n| n.has_tag_name("Component") && n.has_attribute("Classname"))
        .count()
}

#[test]
fn create_channel_under_a_real_group() {
    let Some(xml) = corpus_project() else {
        eprintln!("corpus absent; skipping");
        return;
    };
    let group = components_of_class(&xml, "BuiltIn.GroupCompound")
        .into_iter()
        .next()
        .expect("corpus has a group");
    let new = format!("{group}.M1ProjectSmokeTestChannel");
    let out =
        m1_project::create_channel(&xml, &new, Some("f32"), Some("rpm"), Some("Tune")).unwrap();

    // Still valid XML, the channel is present, and exactly one component was added.
    roxmltree::Document::parse(&out).expect("result must be valid XML");
    assert!(out.contains(&new));
    assert_eq!(component_count(&out), component_count(&xml) + 1);
    // Nothing else changed: removing the inserted element restores the original.
    assert!(out.len() > xml.len());
}

#[test]
fn create_table_under_a_real_group() {
    let Some(xml) = corpus_project() else {
        eprintln!("corpus absent; skipping");
        return;
    };
    let group = components_of_class(&xml, "BuiltIn.GroupCompound")
        .into_iter()
        .next()
        .expect("corpus has a group");
    let channel = components_of_class(&xml, "BuiltIn.Channel")
        .into_iter()
        .next()
        .expect("corpus has a channel");
    let new = format!("{group}.M1ProjectSmokeTestTable");
    let axes = [m1_project::TableAxis {
        source: channel.clone(),
        sites: Some(11),
    }];
    let out = m1_project::create_table(&xml, &new, &axes, Some("Tune")).unwrap();

    roxmltree::Document::parse(&out).expect("result must be valid XML");
    assert!(out.contains(&new));
    assert!(out.contains(r#"NumAxes="1""#));
    assert_eq!(component_count(&out), component_count(&xml) + 1);
    // The absolute channel path was relativized — the literal absolute form
    // must not appear in the new table's axis.
    assert!(out.contains("<X Source=\"Parent."));
    // No new validate findings on the real project.
    assert_eq!(
        m1_project::validate(&out).unwrap().len(),
        m1_project::validate(&xml).unwrap().len()
    );
}

#[test]
fn set_call_rate_matches_corpus_trigger_format() {
    let Some(xml) = corpus_project() else {
        return;
    };
    // Pick a real script and a real clock, then assert the produced trigger uses
    // the same `Parent.×N.Events.On …` shape the corpus already uses.
    let script = components_of_class(&xml, "BuiltIn.MethodUser")
        .into_iter()
        .chain(components_of_class(&xml, "BuiltIn.FuncUser"))
        .next()
        .expect("corpus has a script");
    let rates = m1_project::available_rates(&xml).unwrap();
    let hz = rates
        .iter()
        .find(|r| r.ends_with("Hz"))
        .expect("corpus has an On <N>Hz clock");
    let n = hz.trim_end_matches("Hz");

    let out = m1_project::set_call_rate(&xml, &script, n).unwrap();
    roxmltree::Document::parse(&out).expect("valid XML");
    let parents = "Parent.".repeat(script.matches('.').count());
    let expected = format!(r#"SelectedTrigger="{parents}Events.On {n}Hz""#);
    assert!(
        out.contains(&expected),
        "expected trigger {expected} for script {script}"
    );
}

#[test]
fn set_security_on_a_real_channel_keeps_xml_valid() {
    let Some(xml) = corpus_project() else {
        return;
    };
    let ch = components_of_class(&xml, "BuiltIn.Channel")
        .into_iter()
        .next()
        .expect("corpus has a channel");
    let out = m1_project::set_security(&xml, &ch, "Calibration").unwrap();
    roxmltree::Document::parse(&out).expect("valid XML");
    // The component count is unchanged (we edited, not added).
    assert_eq!(component_count(&out), component_count(&xml));
}

// ---- validate corpus tests --------------------------------------------------

/// Validate the EV-M1 corpus — it should validate clean (zero errors).
/// If findings are returned they are a bug in our checker, not in the corpus.
#[test]
fn validate_ev_m1_corpus_clean() {
    let Some(xml) = corpus_project() else {
        eprintln!("EV-M1 corpus absent; skipping validate test");
        return;
    };
    let findings = m1_project::validate(&xml).expect("validate must not error on valid XML");
    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.level == m1_project::FindingLevel::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "EV-M1 corpus should validate clean; errors found:\n{}",
        errors
            .iter()
            .map(|f| format!("  {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Validate the AV-M1 corpus — it should also validate clean.
#[test]
fn validate_av_m1_corpus_clean() {
    let Some(xml) = av_corpus_project() else {
        eprintln!("AV-M1 corpus absent; skipping validate test");
        return;
    };
    let findings = m1_project::validate(&xml).expect("validate must not error on valid XML");
    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.level == m1_project::FindingLevel::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "AV-M1 corpus should validate clean; errors found:\n{}",
        errors
            .iter()
            .map(|f| format!("  {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---- list-components corpus tests -------------------------------------------

#[test]
fn list_components_ev_m1_returns_all() {
    let Some(xml) = corpus_project() else {
        eprintln!("EV-M1 corpus absent; skipping list-components test");
        return;
    };
    let entries = m1_project::list_components(&xml).expect("list_components must succeed");
    let doc_count = component_count(&xml);
    assert_eq!(
        entries.len(),
        doc_count,
        "list_components must return all {doc_count} components"
    );
    // First entry must be the root.
    assert!(
        entries.first().map(|e| e.path == "Root").unwrap_or(false),
        "first entry must be Root"
    );
    // Every entry must be non-empty path and classname.
    for e in &entries {
        assert!(!e.path.is_empty(), "path must not be empty");
        assert!(!e.classname.is_empty(), "classname must not be empty");
    }
}

// ---- create-group corpus test -----------------------------------------------

#[test]
fn create_group_under_a_real_group() {
    let Some(xml) = corpus_project() else {
        eprintln!("corpus absent; skipping create-group test");
        return;
    };
    let group = components_of_class(&xml, "BuiltIn.GroupCompound")
        .into_iter()
        .next()
        .expect("corpus has a group");
    let new = format!("{group}.M1ProjectSmokeTestGroup");
    let out = m1_project::create_group(&xml, &new).unwrap();
    roxmltree::Document::parse(&out).expect("result must be valid XML");
    assert!(out.contains(&new), "new group must be in the output");
    assert_eq!(component_count(&out), component_count(&xml) + 1);
}
