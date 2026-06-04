//! Encoding round-trip: a `.m1prj` MoTeC wrote as Windows-1252 (e.g. a `°`
//! `0xB0` byte in a unit attribute) must (a) read without the `read_to_string`
//! UTF-8 error and (b) be written back in its original encoding — a 1252 `0xB0`
//! stays a single `0xB0` byte, not UTF-8 `0xC2 0xB0`. Regression test for #2.
use m1_workspace::Encoding;
use std::path::PathBuf;
use std::process::Command;

/// A minimal `.m1prj` whose `Root.Engine.Speed` channel carries a `Unit` whose
/// value contains a raw `0xB0` (`°`) byte — i.e. genuine Windows-1252 content.
fn windows1252_project_bytes() -> Vec<u8> {
    let head = b"<?xml version=\"1.0\"?>\n\
<MoTeCM1BuildSession>\n\
 <Project Name=\"T\">\n\
  <ComponentStream>\n\
   <List>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>\n\
    <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Engine\"/>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Speed\">\n\
     <Props Type=\"f32\"><Locale><Default Unit=\""
        .to_vec();
    let tail = b"C\"/></Locale></Props>\n\
    </Component>\n\
    <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Plain\"/>\n\
   </List>\n\
  </ComponentStream>\n\
 </Project>\n\
</MoTeCM1BuildSession>\n"
        .to_vec();
    // Unit value `°C`: a lone 0xB0 (Windows-1252 degree sign) then `C`.
    let mut bytes = head;
    bytes.push(0xB0);
    bytes.extend_from_slice(&tail);
    bytes
}

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("m1project-enc-{}-{name}", std::process::id()));
    p
}

/// The pure library decode/encode used by the binary round-trips a 1252 `0xB0`
/// through an edit without erroring and without transcoding it to UTF-8.
#[test]
fn set_unit_roundtrips_windows1252_degree_byte() {
    let original = windows1252_project_bytes();

    // (a) Tolerant decode does not error on the 0xB0 byte, and reports 1252.
    let (xml, encoding) = m1_workspace::decode_with_encoding(original.clone());
    assert_eq!(encoding, Encoding::Windows1252, "0xB0 forces a 1252 decode");
    assert!(
        xml.contains("\u{00B0}C"),
        "the degree sign decodes to U+00B0"
    );

    // Edit an unrelated component, leaving the °C unit untouched.
    let edited = m1_project::set_unit(&xml, "Root.Engine.Plain", "rpm").unwrap();
    assert!(edited.contains(r#"<Default Unit="rpm"/>"#));
    // The degree-bearing unit is preserved through the edit.
    assert!(edited.contains("\u{00B0}C"), "untouched °C unit survives");

    // (b) Re-encode in the *original* encoding for write-back.
    let out_bytes = m1_workspace::encode(&edited, encoding);
    // A single 0xB0 byte, not UTF-8 0xC2 0xB0.
    assert!(
        out_bytes.windows(2).all(|w| w != [0xC2, 0xB0]),
        "the degree sign must not be transcoded to UTF-8 0xC2 0xB0"
    );
    assert!(
        out_bytes.contains(&0xB0),
        "the Windows-1252 0xB0 degree byte survives the round-trip"
    );
    // Untouched structure preserved.
    let (out_xml, _) = m1_workspace::decode_with_encoding(out_bytes);
    assert!(out_xml.contains(r#"Name="Root.Engine.Speed""#));
    assert!(out_xml.contains(r#"Name="Root.Engine.Plain""#));
    roxmltree::Document::parse(&out_xml).expect("result is valid XML");
}

/// End-to-end through the built binary: `list-rates` reads the 1252 file
/// without the UTF-8 error, and `set-unit` (no `--stdout`) writes it back in
/// Windows-1252 with the 0xB0 byte intact.
#[test]
fn cli_reads_and_writes_back_windows1252() {
    let bin = env!("CARGO_BIN_EXE_m1-project");
    let read_path = tmp_path("read.m1prj");
    std::fs::write(&read_path, windows1252_project_bytes()).unwrap();

    // (a) A read-only command does not hit the "invalid UTF-8" error.
    let list = Command::new(bin)
        .args(["list-rates", "--project"])
        .arg(&read_path)
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "list-rates failed on a Windows-1252 file: {}",
        String::from_utf8_lossy(&list.stderr)
    );

    // (b) An in-place edit re-encodes back to Windows-1252.
    let write_path = tmp_path("write.m1prj");
    std::fs::write(&write_path, windows1252_project_bytes()).unwrap();
    let edit = Command::new(bin)
        .args([
            "set-unit",
            "--component",
            "Root.Engine.Plain",
            "--unit",
            "rpm",
            "--project",
        ])
        .arg(&write_path)
        .output()
        .unwrap();
    assert!(
        edit.status.success(),
        "set-unit failed: {}",
        String::from_utf8_lossy(&edit.stderr)
    );
    let written = std::fs::read(&write_path).unwrap();
    assert!(
        written.contains(&0xB0) && written.windows(2).all(|w| w != [0xC2, 0xB0]),
        "write-back must keep the 0xB0 byte, not transcode to UTF-8"
    );
    assert!(
        m1_workspace::decode(written).contains(r#"<Default Unit="rpm"/>"#),
        "the edit landed"
    );

    let _ = std::fs::remove_file(&read_path);
    let _ = std::fs::remove_file(&write_path);
}
