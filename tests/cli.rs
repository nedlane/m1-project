//! CLI behaviour tests.
use std::path::PathBuf;
use std::process::Command;

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("m1project-cli-{}-{name}", std::process::id()));
    p
}

/// A minimal, valid `.m1prj` for CLI smoke tests.
fn minimal_project() -> &'static str {
    "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession>\n\
 <Project Name=\"T\">\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Engine\"/>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Speed\"/>\n\
    <Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Engine.Update\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Events\"/>\n\
    <Component Classname=\"BuiltIn.EventKernel\" Name=\"Root.Events.On 100Hz\"/>\n\
    <Component Classname=\"BuiltIn.EventKernel\" Name=\"Root.Events.On Startup\"/>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
}

#[test]
fn missing_project_error_names_the_file() {
    let out = Command::new(env!("CARGO_BIN_EXE_m1-project"))
        .args(["list-rates", "--project", "/no/such/dir/Project.m1prj"])
        .output()
        .expect("run m1-project");
    assert!(!out.status.success(), "a missing project must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Project.m1prj"),
        "the error should name the file, got: {err}"
    );
}

#[test]
fn create_group_cli_smoke() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("create_group.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "create-group",
            "--name",
            "Root.Engine.SubSystem",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "create-group failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(r#"Name="Root.Engine.SubSystem""#),
        "group not found in written file"
    );
    roxmltree::Document::parse(&written).expect("written file must be valid XML");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn delete_component_cli_smoke() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("delete_component.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "delete-component",
            "--name",
            "Root.Engine.Speed",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "delete-component failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        !written.contains(r#"Name="Root.Engine.Speed""#),
        "deleted component still in file"
    );
    roxmltree::Document::parse(&written).expect("valid XML after delete");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn delete_component_recursive_flag() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("delete_recursive.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    // Without --recursive, Engine (which has children) must fail.
    let out = Command::new(bin)
        .args(["delete-component", "--name", "Root.Engine", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success(), "should fail without --recursive");

    // With --recursive it succeeds.
    let out2 = Command::new(bin)
        .args([
            "delete-component",
            "--name",
            "Root.Engine",
            "--recursive",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out2.status.success(),
        "delete --recursive failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(!written.contains("Root.Engine"));
    assert!(written.contains("Root.Events"), "Events must be untouched");
    roxmltree::Document::parse(&written).expect("valid XML");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn rename_component_cli_smoke() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("rename_component.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "rename-component",
            "--name",
            "Root.Engine",
            "--new-name",
            "Motor",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rename-component failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(r#"Name="Root.Motor""#),
        "renamed component not found"
    );
    assert!(
        !written.contains(r#"Name="Root.Engine""#),
        "old name still present"
    );
    roxmltree::Document::parse(&written).expect("valid XML after rename");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_clean_project() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("validate_clean.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args(["validate", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "validate failed on a clean project: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 finding(s)") || stdout.contains("0 error(s)"),
        "expected zero findings, got: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_exits_nonzero_on_bad_trigger() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let prj = "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession><Project Name=\"T\"><ComponentStream><List>\n\
<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
<Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Script\">\n\
 <Props SelectedTrigger=\"Parent.Events.On 999Hz\"/>\n\
</Component>\n\
</List></ComponentStream></Project></MoTeCM1BuildSession>\n";
    let path = tmp_path("validate_bad.m1prj");
    std::fs::write(&path, prj).unwrap();

    let out = Command::new(bin)
        .args(["validate", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "validate should exit non-zero for bad trigger"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ERROR"),
        "expected ERROR in output, got: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn list_components_cli_human() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("list_components_human.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args(["list-components", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "list-components failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Root"), "Root must appear in output");
    assert!(stdout.contains("Engine"), "Engine must appear");
    assert!(stdout.contains("Speed"), "Speed must appear");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn list_components_cli_json() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("list_components_json.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args(["list-components", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "list-components --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Must be a JSON array.
    assert!(stdout.trim_start().starts_with('['), "must start with [");
    assert!(stdout.trim_end().ends_with(']'), "must end with ]");
    assert!(stdout.contains(r#""path""#), "must have path key");
    assert!(stdout.contains(r#""classname""#), "must have classname key");
    assert!(
        stdout.contains("Root.Engine.Speed"),
        "must contain channel path"
    );

    let _ = std::fs::remove_file(&path);
}
