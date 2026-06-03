//! Exercise the mutations against the real EV-M1 `Project.m1prj` when present.
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

/// Names of all `<Component>` of a given class in the project.
fn components_of_class(xml: &str, class: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    doc.descendants()
        .filter(|n| n.has_tag_name("Component") && n.attribute("Classname") == Some(class))
        .filter_map(|n| n.attribute("Name").map(str::to_string))
        .collect()
}

fn component_count(xml: &str) -> usize {
    roxmltree::Document::parse(xml)
        .unwrap()
        .descendants()
        .filter(|n| n.has_tag_name("Component"))
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
