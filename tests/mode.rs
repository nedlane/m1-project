//! Permission-mode preservation: an in-place edit must not widen the project
//! file's mode. A tightened `Project.m1prj` (e.g. `0o600`) must keep that mode
//! after any edit — the atomic write previously created its temp file with the
//! umask default and renamed it over the target, silently widening `0o600` to
//! `0o664`. Regression test; fixed by switching to `m1_workspace::atomic_write`,
//! which `chmod`s the temp to the existing file's mode before the rename.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// A minimal, valid `.m1prj` with a couple of editable components.
fn project_xml() -> &'static str {
    "<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession>\n\
 <Project Name=\"T\">\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Engine\"/>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Plain\"/>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
}

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("m1project-mode-{}-{name}", std::process::id()));
    p
}

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Editing an existing `0o600` project keeps the mode `0o600`, and the edit lands.
#[test]
fn in_place_edit_preserves_tightened_mode() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("edit.m1prj");
    std::fs::write(&path, project_xml()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(mode_of(&path), 0o600, "precondition: file starts at 0o600");

    let edit = Command::new(bin)
        .args([
            "set-unit",
            "--component",
            "Root.Engine.Plain",
            "--unit",
            "rpm",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        edit.status.success(),
        "set-unit failed: {}",
        String::from_utf8_lossy(&edit.stderr)
    );

    assert_eq!(
        mode_of(&path),
        0o600,
        "an in-place edit must preserve the file's 0o600 mode, not widen it"
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains(r#"<Default Unit="rpm"/>"#),
        "the edit landed: {written}"
    );

    let _ = std::fs::remove_file(&path);
}

/// An ordinary in-place edit (umask-default source mode) still writes a valid
/// file with the edit applied — the mode-preserving write must not regress the
/// normal write path.
#[test]
fn edit_writes_a_valid_file_with_default_mode() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let path = tmp_path("new.m1prj");
    std::fs::write(&path, project_xml()).unwrap();

    let edit = Command::new(bin)
        .args([
            "set-unit",
            "--component",
            "Root.Engine.Plain",
            "--unit",
            "kph",
            "--project",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        edit.status.success(),
        "set-unit failed: {}",
        String::from_utf8_lossy(&edit.stderr)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    roxmltree::Document::parse(&written).expect("written file is valid XML");
    assert!(
        written.contains(r#"<Default Unit="kph"/>"#),
        "the edit landed"
    );

    let _ = std::fs::remove_file(&path);
}
