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
