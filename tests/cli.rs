//! CLI behaviour: errors must name the file the user gave (#15).
use std::process::Command;

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
