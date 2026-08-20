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
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Speed\"><Props Security=\"Tune\"/></Component>\n\
    <Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Engine.Update\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Events\"/>\n\
    <Component Classname=\"BuiltIn.EventKernel\" Name=\"Root.Events.On 100Hz\"/>\n\
    <Component Classname=\"BuiltIn.EventKernel\" Name=\"Root.Events.On Startup\"/>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
}

fn dbc_project(md5: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession><Project Name=\"T\"><ComponentStream>\
<List>\
<Component Classname=\"BuiltIn.CAN.DBCRoot\" Name=\"DBC\"/>\
<Component Classname=\"BuiltIn.CAN.DBC\" MD5=\"{md5}\" Name=\"DBC.Sample\"/>\
</List>\
<Organisation><Component Name=\"DBC\"><Component Name=\"Sample\"/></Component></Organisation>\
</ComponentStream></Project></MoTeCM1BuildSession>\n"
    )
}

fn dbc_file(md5: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n\
<DBC><ComponentStream>\
<List><Component Classname=\"BuiltIn.CAN.DBC\" MD5=\"{md5}\" Name=\"Sample\"/></List>\
<Organisation><Component Name=\"Sample\"/></Organisation>\
</ComponentStream></DBC>\n"
    )
}

fn run_validate(project: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_m1-project"))
        .args(["validate", "--project"])
        .arg(project)
        .output()
        .unwrap()
}

#[test]
fn validate_finds_dbc_in_workspace_source_directory() {
    let root = tmp_path("dbc_workspace_source");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::create_dir_all(root.join("can")).unwrap();
    std::fs::write(root.join("m1-tools.toml"), "[dbc]\nsrc_dir = \"can\"\n").unwrap();
    std::fs::write(
        root.join("proj/Project.m1prj"),
        dbc_project("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )
    .unwrap();
    std::fs::write(
        root.join("can/Sample.m1dbc"),
        dbc_file("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    )
    .unwrap();

    let out = run_validate(&root.join("proj/Project.m1prj"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "the MD5 drift must fail validation");
    assert!(
        stdout.contains("`can/Sample.m1dbc` internal MD5"),
        "the finding must prove the workspace source was read and name it: {stdout}"
    );
    assert!(!stdout.contains("dbc/Sample.m1dbc is missing"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn validate_prefers_the_selected_projects_dbc_file() {
    let root = tmp_path("dbc_project_scope");
    let _ = std::fs::remove_dir_all(&root);
    for version in ["01.00", "02.00"] {
        std::fs::create_dir_all(root.join(format!("UQR-EV/{version}/dbc"))).unwrap();
    }
    std::fs::write(root.join("m1-tools.toml"), "[format]\nline_width = 100\n").unwrap();
    std::fs::write(
        root.join("UQR-EV/02.00/Project.m1prj"),
        dbc_project("22222222222222222222222222222222"),
    )
    .unwrap();
    std::fs::write(
        root.join("UQR-EV/01.00/dbc/Sample.m1dbc"),
        dbc_file("11111111111111111111111111111111"),
    )
    .unwrap();
    std::fs::write(
        root.join("UQR-EV/02.00/dbc/Sample.m1dbc"),
        dbc_file("22222222222222222222222222222222"),
    )
    .unwrap();

    let out = run_validate(&root.join("UQR-EV/02.00/Project.m1prj"));
    assert!(
        out.status.success(),
        "validation must not read the other project's same-named source: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn validate_refuses_ambiguous_workspace_dbc_files() {
    let root = tmp_path("dbc_ambiguous");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(root.join("m1-tools.toml"), "").unwrap();
    std::fs::write(
        root.join("proj/Project.m1prj"),
        dbc_project("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )
    .unwrap();
    let source = dbc_file("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    std::fs::write(root.join("a/Sample.m1dbc"), &source).unwrap();
    std::fs::write(root.join("b/Sample.m1dbc"), &source).unwrap();

    let out = run_validate(&root.join("proj/Project.m1prj"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "ambiguous sources must fail validation"
    );
    assert!(
        stdout.contains("cannot be located unambiguously"),
        "{stdout}"
    );
    assert!(stdout.contains("a/Sample.m1dbc"), "{stdout}");
    assert!(stdout.contains("b/Sample.m1dbc"), "{stdout}");

    let _ = std::fs::remove_dir_all(&root);
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
    // A genuinely-clean project needs its script component's backing `.m1scr` to
    // exist and carry code (the CLI's "missing code" check is file-aware), so use
    // a dedicated dir with a populated Scripts/ rather than a bare temp file.
    let dir = tmp_path("validate_clean_dir");
    let scripts = dir.join("Scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let path = dir.join("Project.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();
    // minimal_project()'s MethodUser is Root.Engine.Update → Engine.Update.m1scr.
    std::fs::write(scripts.join("Engine.Update.m1scr"), "Speed = 1;\n").unwrap();

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

    let _ = std::fs::remove_dir_all(&dir);
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

/// A minimal project carrying a `FileFormat` header and one typed signature, for
/// the `format` subcommand tests.
fn project_with_format(file_format: u32, product_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession ProductName=\"M1Build (x64)\" ProductVersion=\"{product_version}\">\n\
 <Project FileFormat=\"{file_format}\" Name=\"UQR-EV\">\n\
  <System VersionMajor=\"1\" VersionMinor=\"4\" VersionRelease=\"0\" VersionBuild=\"0108\"/>\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
    )
}

#[test]
fn format_cli_reports_current_version() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("format_report.m1prj");
    std::fs::write(&path, project_with_format(10108, "1.4.4.981")).unwrap();

    let out = Command::new(bin)
        .args(["format", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success(), "format report failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("FileFormat:") && stdout.contains("10108"),
        "got: {stdout}"
    );
    assert!(
        stdout.contains("1.4.4.981"),
        "writer version missing: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn format_cli_converts_and_writes() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("format_convert.m1prj");
    std::fs::write(&path, project_with_format(10108, "1.4.4.981")).unwrap();

    let out = Command::new(bin)
        .args(["format", "--target", "10109", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "format convert failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("FileFormat=\"10109\""),
        "not upgraded: {written}"
    );
    assert!(written.contains("ProductVersion=\"1.4.5.556\""));
    roxmltree::Document::parse(&written).expect("valid XML after convert");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_max_format_gate() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("format_gate.m1prj");
    std::fs::write(&path, project_with_format(10109, "1.4.5.556")).unwrap();

    // A 10109 project fails --max-format 10108 …
    let out = Command::new(bin)
        .args(["validate", "--max-format", "10108", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success(), "gate should fail a too-new format");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("exceeds the maximum"), "got: {stdout}");

    // … and passes --max-format 10109.
    let ok = Command::new(bin)
        .args(["validate", "--max-format", "10109", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&ok.stdout).contains("exceeds the maximum"),
        "at-limit format must not be gated"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_flags_invalid_object_name_1132() {
    // A component whose name contains a reserved keyword (`and`) is what M1-Build
    // rejects on open with Error 1132; `validate` must catch it and exit non-zero,
    // where before it reported the project clean (the CI-gate false negative).
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let prj = "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession><Project Name=\"UQR-EV\"><ComponentStream><List>\n\
<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.CAN\"/>\n\
<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.CAN.Receive IRTS and SGAMP 100Hz\"/>\n\
</List></ComponentStream></Project></MoTeCM1BuildSession>\n";
    let path = tmp_path("validate_bad_name.m1prj");
    std::fs::write(&path, prj).unwrap();

    let out = Command::new(bin)
        .args(["validate", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "validate should exit non-zero for an invalid object name"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1132") && stdout.contains("reserved keyword"),
        "expected a 1132 invalid-name finding, got: {stdout}"
    );
    // The <Project> Name `UQR-EV` (a hyphen) must NOT be flagged — it is not an
    // object name.
    assert!(
        !stdout.contains("UQR-EV` is not"),
        "the project name must not be flagged: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_flags_missing_code() {
    // A script component whose backing .m1scr is empty is M1-Build's "Missing
    // code" error; the CLI's file-aware check must surface it and exit non-zero.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let dir = tmp_path("validate_missing_code_dir");
    let scripts = dir.join("Scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let path = dir.join("Project.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();
    // Engine.Update.m1scr present but EMPTY → "missing code".
    std::fs::write(scripts.join("Engine.Update.m1scr"), "   \n").unwrap();

    let out = Command::new(bin)
        .args(["validate", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "validate should exit non-zero when a script has no code"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("missing code") && stdout.contains("Root.Engine.Update"),
        "expected a missing-code finding, got: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
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

/// A `.m1prj` declaring its security groups inline, including a custom `PDM`
/// group — the shape the real AV-M1 project uses.
fn project_with_security_roles() -> &'static str {
    "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession>\n\
 <Project Name=\"T\">\n\
  <SecurityMgr>\n\
   <SecurityRoles>\n\
    <SecurityRole Name=\"Tune\"/>\n\
    <SecurityRole Name=\"Calibration\"/>\n\
    <SecurityRole Name=\"PDM\"/>\n\
   </SecurityRoles>\n\
  </SecurityMgr>\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Sig\"><Props Type=\"f32\"/></Component>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
}

#[test]
fn list_security_cli_human_falls_back_to_defaults() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("list_security_human.m1prj");
    // minimal_project has no <SecurityMgr> => the four standard groups.
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args(["list-security", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "list-security failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for g in ["Tune", "Calibration", "Master Calibration", "Resource"] {
        assert!(
            stdout.contains(g),
            "default group {g} must appear: {stdout}"
        );
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn list_security_cli_json_surfaces_custom_group() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("list_security_json.m1prj");
    std::fs::write(&path, project_with_security_roles()).unwrap();

    let out = Command::new(bin)
        .args(["list-security", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "list-security --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_start().starts_with('['), "must start with [");
    assert!(stdout.trim_end().ends_with(']'), "must end with ]");
    // The project-specific custom group a hard-coded editor list would miss.
    assert!(
        stdout.contains(r#""PDM""#),
        "custom PDM group must be surfaced: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_comment_cli_writes_cdata_and_clears() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("set_comment.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "set-comment",
            "--component",
            "Root.Engine.Speed",
            "--comment",
            "Wheel speed, NDD filtered",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "set-comment failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    // M1-Build's serialiser shape: CDATA on its own line.
    assert!(
        written.contains("<Comment>\n<![CDATA[Wheel speed, NDD filtered]]>"),
        "comment CDATA not found: {written}"
    );
    roxmltree::Document::parse(&written).expect("valid XML");

    // Read-back through list-components --json.
    let out = Command::new(bin)
        .args(["list-components", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    let json = String::from_utf8_lossy(&out.stdout);
    assert!(
        json.contains(r#""comment":"Wheel speed, NDD filtered""#),
        "comment must round-trip through list-components --json: {json}"
    );

    // Empty text clears back to the placeholder.
    let out = Command::new(bin)
        .args([
            "set-comment",
            "--component",
            "Root.Engine.Speed",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("<Comment/>"), "cleared: {written}");
    assert!(
        !written.contains("CDATA"),
        "no CDATA after clear: {written}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn create_reference_cli_smoke() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("create_reference.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    // Bare reference: the corpus-majority self-closing shape.
    let out = Command::new(bin)
        .args([
            "create-reference",
            "--name",
            "Root.Engine.Speed Alias",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "create-reference failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(
            r#"<Component Classname="BuiltIn.Reference" Name="Root.Engine.Speed Alias"/>"#
        ),
        "self-closing reference not found: {written}"
    );
    assert!(
        !written.contains("AutoCreated"),
        "must never emit M1-Build's AutoCreated marker"
    );

    // Explicit target → the Props TargetCreation form.
    let out = Command::new(bin)
        .args([
            "create-reference",
            "--name",
            "Root.Engine.Targeted",
            "--target",
            "This.Value",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "targeted create-reference failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(r#"<Props TargetCreation="AutoParam" Target="This.Value"/>"#),
        "targeted reference props not found: {written}"
    );
    roxmltree::Document::parse(&written).expect("valid XML");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_json_emits_machine_findings() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("validate_json.m1prj");
    // A project with a dangling SelectedTrigger so validate has something to say.
    let xml = minimal_project().replace(
        r#"<Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Update"/>"#,
        r#"<Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Update"><Props SelectedTrigger="Parent.Parent.Events.On 999Hz"/></Component>"#,
    );
    std::fs::write(&path, xml).unwrap();

    let out = Command::new(bin)
        .args(["validate", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim_start().starts_with('['),
        "must be a JSON array: {stdout}"
    );
    assert!(
        stdout.contains(r#""level":"#) && stdout.contains(r#""path":"#),
        "findings must carry level/path/message: {stdout}"
    );
    // Output must parse as JSON.
    serde_json_sanity(&stdout);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_cli_rejects_invalid_project_root() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("validate_invalid_root.m1prj");
    std::fs::write(&path, "<?xml version=\"1.0\"?><NotAM1Project/>").unwrap();

    let out = Command::new(bin)
        .args(["validate", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("MoTeCM1BuildSession"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_storage_class_cli_supports_dry_run_and_json_readback() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("set_storage_class.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let preview = Command::new(bin)
        .args([
            "--dry-run",
            "set-storage-class",
            "--component",
            "Root.Engine.Speed",
            "--storage-class",
            "flash",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains(r#"Storage="Flash""#));
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("Storage=\"Flash\"")
    );

    let write = Command::new(bin)
        .args([
            "set-storage-class",
            "--component",
            "Root.Engine.Speed",
            "--storage-class",
            "flash",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );

    let listed = Command::new(bin)
        .args(["list-components", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(stdout.contains(r#""storage_class":"flash""#), "{stdout}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_json_emits_m1build_error_codes() {
    // The README markets `validate` as referencing M1-Build's error numbers; the
    // --json output must therefore carry a machine-readable `code` per finding so
    // a CI consumer can triage by error number without parsing the message (#83).
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("validate_codes.m1prj");
    // A channel with NO <Props Security> → "no security group selected"
    // (M1-Build Error 1601). Plus an <Organisation> view that omits that same
    // channel → "absent from the <Organisation> view" (Error 1338). The rest of
    // the List is mirrored in the view so only the bare channel is flagged twice.
    let xml = "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession>\n\
 <Project Name=\"T\">\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Engine\"/>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Bare\"/>\n\
   </List>\n\
   <Organisation>\n\
    <Component Name=\"Root\">\n\
     <Component Name=\"Engine\"/>\n\
    </Component>\n\
   </Organisation>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n";
    std::fs::write(&path, xml).unwrap();

    let out = Command::new(bin)
        .args(["validate", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json_sanity(&stdout);

    // The 1601 (no security) and 1338 (org-tree mismatch) codes must appear as
    // structured number fields, not buried in free text.
    assert!(
        stdout.contains(r#""code":1601"#),
        "missing-security finding must carry code 1601: {stdout}"
    );
    assert!(
        stdout.contains(r#""code":1338"#),
        "org-tree-mismatch finding must carry code 1338: {stdout}"
    );

    let _ = std::fs::remove_file(&path);
}

/// m1-project deliberately has no serde dependency; sanity-parse the JSON with
/// a tiny structural check instead (balanced brackets, no trailing comma).
fn serde_json_sanity(s: &str) {
    let t = s.trim();
    assert!(t.starts_with('[') && t.ends_with(']'), "array shape: {t}");
    assert!(!t.contains(",\n]"), "no trailing comma: {t}");
}

#[test]
fn dry_run_prints_diff_and_stdout_prints_xml() {
    // #51: --dry-run is a preview (unified diff, file untouched); --stdout is
    // output routing (raw XML, file untouched).
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("dryrun_vs_stdout.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let args = [
        "create-channel",
        "--name",
        "Root.Engine.Temp",
        "--type",
        "f32",
        "--project",
    ];
    let dry = Command::new(bin)
        .args(args)
        .arg(&path)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(dry.status.success());
    let dry_out = String::from_utf8_lossy(&dry.stdout);
    assert!(
        dry_out.contains("+") && dry_out.contains("Root.Engine.Temp") && dry_out.contains("@@"),
        "--dry-run must print a unified diff, got: {dry_out}"
    );
    assert!(
        !dry_out.trim_start().starts_with("<?xml"),
        "--dry-run must not dump raw XML"
    );

    let raw = Command::new(bin)
        .args(args)
        .arg(&path)
        .arg("--stdout")
        .output()
        .unwrap();
    assert!(raw.status.success());
    let raw_out = String::from_utf8_lossy(&raw.stdout);
    assert!(
        raw_out.trim_start().starts_with("<?xml"),
        "--stdout must print the raw XML result, got: {raw_out}"
    );

    // Neither touched the file.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), minimal_project());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn json_escapes_control_characters() {
    // #50: a multiline comment (CDATA preserves newlines) must come out of
    // `list-components --json` as an `\n` escape, not a raw control char
    // inside the string literal — which strict JSON parsers reject.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("json_escape.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let set = Command::new(bin)
        .args([
            "set-comment",
            "--component",
            "Root.Engine.Speed",
            "--comment",
            "line one\nline two",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "set-comment failed: {}",
        String::from_utf8_lossy(&set.stderr)
    );

    let out = Command::new(bin)
        .args(["list-components", "--json", "--project"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("line one\\nline two"),
        "newline must be escaped as \\n inside the JSON string, got: {stdout}"
    );
    assert!(
        !stdout.contains("line one\nline two"),
        "no raw newline may appear inside a JSON string literal"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn rename_rolls_back_files_when_a_rename_fails() {
    // #49: file renames happen before the XML write; a mid-loop failure rolls
    // back completed renames and leaves the project XML untouched.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let dir = tmp_path("rename_tx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Scripts")).unwrap();
    let path = dir.join("Project.m1prj");
    // Two script components under the group being renamed.
    let xml = minimal_project().replace(
        "<Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Engine.Update\"/>",
        "<Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Engine.Update\"/>\n    \
         <Component Classname=\"BuiltIn.MethodUser\" Name=\"Root.Engine.Apply\"/>",
    );
    std::fs::write(&path, &xml).unwrap();
    std::fs::write(dir.join("Scripts/Engine.Update.m1scr"), "/* a */\n").unwrap();
    std::fs::write(dir.join("Scripts/Engine.Apply.m1scr"), "/* b */\n").unwrap();
    // Make the SECOND rename fail: its destination exists as a non-empty
    // directory, which fs::rename cannot replace.
    std::fs::create_dir_all(dir.join("Scripts/Motor.Apply.m1scr/block")).unwrap();

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
        !out.status.success(),
        "rename must fail when a file rename fails: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    // XML untouched, first rename rolled back.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        xml,
        "XML must be untouched"
    );
    assert!(
        dir.join("Scripts/Engine.Update.m1scr").exists(),
        "completed rename must be rolled back"
    );
    assert!(
        !dir.join("Scripts/Motor.Update.m1scr").exists(),
        "no renamed file may remain"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rename_refuses_existing_destination_file() {
    // An unrelated (orphan) .m1scr already sits at a rename destination. On
    // platforms where fs::rename replaces the destination, proceeding would
    // silently destroy its bytes before the XML write — unrecoverable by
    // rollback. The whole rename must be refused up front: destination bytes,
    // source files, and the XML all stay untouched.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let dir = tmp_path("rename_dest_exists");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Scripts")).unwrap();
    let path = dir.join("Project.m1prj");
    let xml = minimal_project();
    std::fs::write(&path, xml).unwrap();
    std::fs::write(dir.join("Scripts/Engine.Update.m1scr"), "/* source */\n").unwrap();
    // The orphan occupying the destination path.
    std::fs::write(dir.join("Scripts/Motor.Update.m1scr"), "/* precious */\n").unwrap();

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
        !out.status.success(),
        "rename must refuse an existing destination: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Motor.Update.m1scr"),
        "error must name the occupied destination, got: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Scripts/Motor.Update.m1scr")).unwrap(),
        "/* precious */\n",
        "existing destination must remain byte-for-byte unchanged"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Scripts/Engine.Update.m1scr")).unwrap(),
        "/* source */\n",
        "source must remain in place"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        xml,
        "XML must be untouched"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_function_leaves_xml_untouched_when_backing_file_fails() {
    // The backing .m1scr must be staged BEFORE the XML write: when file
    // creation fails (here the Scripts path is occupied by a regular file so
    // the directory cannot be created), the project XML must not have changed —
    // no half-committed component pointing at a script that was never created.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let dir = tmp_path("create_fn_tx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Project.m1prj");
    let xml = minimal_project();
    std::fs::write(&path, xml).unwrap();
    // Occupy the Scripts directory path with a FILE: create_dir_all must fail.
    std::fs::write(dir.join("Scripts"), "not a directory").unwrap();

    let out = Command::new(bin)
        .args([
            "create-scheduled-function",
            "--name",
            "Root.Engine.Compute",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "create must fail when the backing file cannot be created: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        xml,
        "XML must be untouched after a backing-file failure"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_parameter_cli_smoke() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("create_parameter.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "create-parameter",
            "--name",
            "Root.Engine.Gain",
            "--type",
            "f32",
            "--unit",
            "ratio",
            "--security",
            "Tune",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "create-parameter failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(r#"Name="Root.Engine.Gain""#),
        "parameter not found in written file"
    );
    assert!(
        written.contains("BuiltIn.Parameter"),
        "classname BuiltIn.Parameter not found"
    );
    roxmltree::Document::parse(&written).expect("written file must be valid XML");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_validation_rejects_non_finite_min() {
    // clap parses "NaN" as a valid f64, so the bound reaches set_validation; a
    // non-finite bound would serialise to a garbage token M1-Build can't read
    // back, so the command must fail and leave the project untouched.
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("set_validation_nan.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "set-validation",
            "--component",
            "Root.Engine.Speed",
            "--type",
            "MinMax",
            "--min",
            "NaN",
            "--max",
            "1.0",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a non-finite validation bound must fail"
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        !written.contains("ValMin=") && !written.contains("NaN"),
        "the rejected bound must not be written: {written}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_display_range_rejects_non_finite_max() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("set_display_range_inf.m1prj");
    std::fs::write(&path, minimal_project()).unwrap();

    let out = Command::new(bin)
        .args([
            "set-display-range",
            "--component",
            "Root.Engine.Speed",
            "--min",
            "0.0",
            "--max",
            "inf",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a non-finite display bound must fail"
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        !written.contains("Max=") && !written.contains("inf"),
        "the rejected bound must not be written: {written}"
    );

    let _ = std::fs::remove_file(&path);
}
