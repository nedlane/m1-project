use m1_project::*;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn project(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<MoTeCM1BuildSession><Project FileFormat=\"10109\"><ComponentStream><List><Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root\"/>{body}</List></ComponentStream></Project></MoTeCM1BuildSession>"
    )
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(xml: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "m1-project-hardening-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("Project.m1prj"), xml).unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_m1-project"))
            .args(args)
            .arg("--project")
            .arg(self.0.join("Project.m1prj"))
            .output()
            .unwrap()
    }
    fn unchanged(&self, xml: &str) {
        assert_eq!(
            std::fs::read(self.0.join("Project.m1prj")).unwrap(),
            xml.as_bytes()
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn newly_introduced_names_follow_the_existing_validator() {
    let xml = project("");
    for name in ["1bad", "Bad  Name", "if", "Bad/Name", "Bad\\Name", "C:Name"] {
        assert!(
            create_group(&xml, &format!("Root.{name}")).is_err(),
            "{name}"
        );
    }
    let old = project("<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.1bad\"/>");
    let (fixed, _) = rename_component(&old, "Root.1bad", "Good").unwrap();
    assert!(
        !validate(&fixed)
            .unwrap()
            .iter()
            .any(|f| f.code == Some(1132))
    );
}

#[test]
fn nested_table_axes_are_protected_rewritten_and_validated() {
    let xml = project(
        "<Component Classname=\"BuiltIn.Channel\" Name=\"Root.C\"><Props Security=\"Tune\"/></Component>",
    );
    let axes = ["Root.C", "Parent.C.Value", "Parent.C.Resource"].map(|s| TableAxis {
        source: s.into(),
        sites: Some(2),
    });
    let xml = create_table(&xml, "Root.T", &axes, Some("Tune")).unwrap();
    assert!(delete_component(&xml, "Root.C", false, false).is_err());
    let forced = delete_component(&xml, "Root.C", false, true).unwrap();
    assert!(!forced.contains("Name=\"Root.C\""));
    // Unknown inherited members under Root remain deliberately conservative.
    // No-root sources must be collected and diagnosed on all three axes.
    let dangling = forced.replace("Parent.C", "Nowhere.C");
    assert_eq!(
        validate(&dangling)
            .unwrap()
            .iter()
            .filter(|f| f.message.contains("dangling reference"))
            .count(),
        3
    );
    let (renamed, _) = rename_component(&xml, "Root.C", "D").unwrap();
    let doc = roxmltree::Document::parse(&renamed).unwrap();
    let sources: Vec<_> = doc
        .descendants()
        .filter_map(|n| n.attribute("Source"))
        .collect();
    assert_eq!(sources, ["Parent.D", "Parent.D.Value", "Parent.D.Resource"]);
    assert!(
        !validate(&renamed)
            .unwrap()
            .iter()
            .any(|f| f.message.contains("dangling reference"))
    );
}

#[test]
fn references_inside_a_renamed_subtree_keep_their_targets() {
    let xml = project(
        "<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.A\"/><Component Classname=\"BuiltIn.Channel\" Name=\"Root.A.C\"><Props Security=\"Tune\"/></Component><Component Classname=\"BuiltIn.Reference\" Name=\"Root.A.Absolute\"><Props Target=\"Root.A.C.Value\"/></Component><Component Classname=\"BuiltIn.Reference\" Name=\"Root.A.Relative\"><Props Target=\"This.C\"/></Component>",
    );
    let (out, _) = rename_component(&xml, "Root.A", "B").unwrap();
    assert!(out.contains("Target=\"Root.B.C.Value\""));
    assert!(out.contains("Target=\"This.C\""));
    assert!(
        !validate(&out)
            .unwrap()
            .iter()
            .any(|f| f.message.contains("dangling reference"))
    );
}

#[test]
fn custom_script_filename_moves_with_its_component() {
    let xml = project(
        "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\" Filename=\"Custom.m1scr\"/>",
    );
    let f = Fixture::new(&xml);
    std::fs::create_dir(f.0.join("Scripts")).unwrap();
    std::fs::write(f.0.join("Scripts/Custom.m1scr"), "return 123;").unwrap();
    for flag in ["--dry-run", "--stdout"] {
        let result = f.run(&[
            "rename-component",
            "--name",
            "Root.F",
            "--new-name",
            "G",
            flag,
        ]);
        assert!(result.status.success(), "{result:?}");
        f.unchanged(&xml);
        assert!(f.0.join("Scripts/Custom.m1scr").exists());
    }
    let result = f.run(&["rename-component", "--name", "Root.F", "--new-name", "G"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(f.0.join("Scripts/G.m1scr")).unwrap(),
        "return 123;"
    );
    assert!(!f.0.join("Scripts/Custom.m1scr").exists());
}

#[test]
fn unsafe_existing_script_filenames_are_refused_before_editing() {
    for filename in [
        "../outside.m1scr",
        "/outside.m1scr",
        "C:\\outside.m1scr",
        "..\\outside.m1scr",
    ] {
        let xml = project(&format!(
            "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\" Filename=\"{filename}\"/>"
        ));
        assert!(rename_component(&xml, "Root.F", "G").is_err(), "{filename}");
    }
}

#[cfg(unix)]
#[test]
fn script_creation_rejects_links_without_touching_their_targets() {
    use std::os::unix::fs::symlink;
    for dangling in [true, false] {
        let xml = project("");
        let f = Fixture::new(&xml);
        std::fs::create_dir(f.0.join("Scripts")).unwrap();
        let target = f.0.join("outside");
        if !dangling {
            std::fs::write(&target, "keep").unwrap();
        }
        symlink(&target, f.0.join("Scripts/F.m1scr")).unwrap();
        let result = f.run(&["create-function", "--name", "Root.F"]);
        assert!(!result.status.success(), "{result:?}");
        f.unchanged(&xml);
        assert_eq!(target.exists(), !dangling);
        if !dangling {
            assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
        }
    }
}

#[cfg(unix)]
#[test]
fn script_directory_links_cannot_redirect_creation() {
    let xml = project("");
    let f = Fixture::new(&xml);
    let outside = Fixture::new(&xml);
    std::os::unix::fs::symlink(&outside.0, f.0.join("Scripts")).unwrap();
    let result = f.run(&["create-function", "--name", "Root.F"]);
    assert!(!result.status.success(), "{result:?}");
    assert!(!outside.0.join("F.m1scr").exists());
    f.unchanged(&xml);
}

#[test]
fn explicit_utf8_declarations_preserve_utf8_bytes() {
    for decl in [
        "<?xml version='1.0' encoding = 'UTF-8'?>".to_owned(),
        "<?xml version='1.0'\nencoding\t=\t'utf-8'?>".to_owned(),
        format!("<?xml version='1.0'{}encoding='UTF-8'?>", " ".repeat(300)),
    ] {
        let xml = project("").replacen("<?xml version=\"1.0\"?>", &decl, 1);
        let f = Fixture::new(&xml);
        let result = f.run(&["set-comment", "--component", "Root", "--comment", "café"]);
        assert!(result.status.success(), "{result:?}");
        let bytes = std::fs::read(f.0.join("Project.m1prj")).unwrap();
        let text = String::from_utf8(bytes).expect("the declaration promises UTF-8");
        assert!(text.contains("café"));
        roxmltree::Document::parse(&text).unwrap();
    }
}

#[test]
fn unsupported_encoding_is_rejected_without_writing() {
    let xml = project("").replacen(
        "<?xml version=\"1.0\"?>",
        "<?xml version='1.0' encoding='ISO-8859-1'?>",
        1,
    );
    let f = Fixture::new(&xml);
    let result = f.run(&["set-comment", "--component", "Root", "--comment", "changed"]);
    assert!(!result.status.success(), "{result:?}");
    f.unchanged(&xml);
}

#[test]
fn unsupported_signature_layouts_fail_without_panicking_or_writing() {
    for signature in [
        "<Signature Name=\"\"/>",
        "<Signature Name=\"\"></Signature>",
    ] {
        let xml = project(&format!(
            "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\">{signature}</Component>"
        ));
        assert!(convert_format(&xml, 10108).is_err());
        let f = Fixture::new(&xml);
        let result = f.run(&["format", "--target", "10108"]);
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
        f.unchanged(&xml);
    }
}

#[test]
fn conversion_preserves_quoted_types_and_open_tag_boundaries() {
    let signature = "\n <Signature Name=\"A > B\">\n  <Returns><Return Type=\"A &quot;B&quot; &amp; C\"><Description><![CDATA[kept]]></Description></Return></Returns>\n </Signature>";
    let xml = project(signature).replace(
        "<MoTeCM1BuildSession>",
        "<MoTeCM1BuildSession Name=\"A > B\">",
    );
    let converted = convert_format(&xml, 10108).unwrap();
    let doc = roxmltree::Document::parse(&converted).unwrap();
    let sig = doc
        .descendants()
        .find(|n| n.has_tag_name("Signature"))
        .unwrap();
    assert_eq!(sig.attribute("Name"), Some("A > B"));
    assert_eq!(sig.attribute("ReturnType"), Some("A \"B\" & C"));
    let restored = convert_format(&converted, 10109).unwrap();
    let doc = roxmltree::Document::parse(&restored).unwrap();
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name("Return"))
            .unwrap()
            .attribute("Type"),
        Some("A \"B\" & C")
    );
    assert!(restored.contains("<![CDATA[kept]]>"));
}

#[test]
fn conversion_refuses_unrecognised_description_payloads() {
    for text in [
        "important plain text",
        "<![CDATA[first]]>lost",
        "<![CDATA[first]]><![CDATA[second]]>",
    ] {
        let xml = project(&format!(
            "\n <Signature Name=\"\">\n  <Returns><Return Type=\"f32\"><Description>{text}</Description></Return></Returns>\n </Signature>"
        ));
        assert!(
            convert_format(&xml, 10108).is_err(),
            "must not discard {text}"
        );
    }
}

#[test]
fn creating_a_script_under_an_unsafe_existing_parent_is_refused() {
    let xml = project("<Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Bad/Parent\"/>");
    assert!(create_function(&xml, "Root.Bad/Parent.F").is_err());
    assert!(create_scheduled_function(&xml, "Root.Bad/Parent.F").is_err());
}

#[test]
fn renaming_a_shared_script_cannot_disconnect_another_component() {
    let xml = project(
        "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\" Filename=\"Shared.m1scr\"/><Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.H\" Filename=\"Shared.m1scr\"/>",
    );
    assert!(rename_component(&xml, "Root.F", "G").is_err());
}

#[test]
fn upgrade_refuses_return_metadata_it_cannot_preserve() {
    for content in [
        "<Signature Name=\"\" ReturnType=\"f32\">\n <Params/>\n</Signature>",
        "<Signature Name=\"\" ReturnType=\"\">\n <ReturnDescription>KEEP THIS</ReturnDescription>\n</Signature>",
        "<Signature Name=\"\" ReturnType=\"\">\n <ReturnDescription><![CDATA[KEEP THIS]]></ReturnDescription>\n</Signature>",
    ] {
        let xml = project(&format!("\n{content}\n")).replace("10109", "10108");
        assert!(convert_format(&xml, 10109).is_err(), "{content}");
        let f = Fixture::new(&xml);
        let result = f.run(&["format", "--target", "10109"]);
        assert!(!result.status.success(), "{result:?}");
        assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
        f.unchanged(&xml);
    }
}

#[test]
fn downgrade_refuses_additional_return_content() {
    for returns in [
        "<Returns><Return Type=\"f32\"><Description><![CDATA[first]]></Description></Return><Return Type=\"bool\"><Description><![CDATA[KEEP SECOND]]></Description></Return></Returns>",
        "<Returns Extra=\"KEEP\"><Return Type=\"f32\"><Description><![CDATA[first]]></Description></Return></Returns>",
        "<Returns><Return Type=\"f32\" Extra=\"KEEP\"><Description><![CDATA[first]]></Description></Return></Returns>",
        "<Returns><Return Type=\"f32\"><Description Extra=\"KEEP\"><![CDATA[first]]></Description></Return></Returns>",
        "<Returns><Return Type=\"f32\"><Description><![CDATA[first]]></Description><Other>KEEP</Other></Return></Returns>",
        "<Returns><!--KEEP--><Return Type=\"f32\"><Description><![CDATA[first]]></Description></Return></Returns>",
    ] {
        let xml = project(&format!(
            "\n<Signature Name=\"\">\n {returns}\n</Signature>\n"
        ));
        assert!(convert_format(&xml, 10108).is_err(), "{returns}");
        let f = Fixture::new(&xml);
        let result = f.run(&["format", "--target", "10108"]);
        assert!(!result.status.success(), "{result:?}");
        f.unchanged(&xml);
    }
}

#[cfg(unix)]
#[test]
fn leaf_project_symlinks_refuse_writes_before_script_side_effects() {
    use std::os::unix::fs::symlink;
    let xml = project(
        "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\" Filename=\"F.m1scr\"/>",
    );
    let commands: &[&[&str]] = &[
        &["rename-component", "--name", "Root.F", "--new-name", "G"],
        &["create-function", "--name", "Root.New"],
        &["create-scheduled-function", "--name", "Root.New"],
        &["create-group", "--name", "Root.New"],
        &["format", "--target", "10108"],
        &["format", "--target", "10109"],
    ];
    for separate_directory in [false, true] {
        let f = Fixture::new(&xml);
        std::fs::create_dir(f.0.join("Scripts")).unwrap();
        std::fs::write(f.0.join("Scripts/F.m1scr"), b"// preserve code\r\n").unwrap();
        let alias = if separate_directory {
            std::fs::create_dir(f.0.join("aliases")).unwrap();
            f.0.join("aliases/Alias.m1prj")
        } else {
            f.0.join("Alias.m1prj")
        };
        symlink(f.0.join("Project.m1prj"), &alias).unwrap();
        for args in commands {
            for preview in [Some("--dry-run"), Some("--stdout"), None] {
                let mut cmd = Command::new(env!("CARGO_BIN_EXE_m1-project"));
                cmd.args(*args).arg("--project").arg(&alias);
                if let Some(flag) = preview {
                    cmd.arg(flag);
                }
                let output = cmd.output().unwrap();
                if preview.is_some() {
                    assert!(output.status.success(), "{args:?}: {output:?}");
                } else {
                    assert!(!output.status.success(), "{args:?}: {output:?}");
                    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
                }
                f.unchanged(&xml);
                assert!(alias.is_symlink());
                assert_eq!(
                    std::fs::read(f.0.join("Scripts/F.m1scr")).unwrap(),
                    b"// preserve code\r\n"
                );
                assert_eq!(std::fs::read_dir(f.0.join("Scripts")).unwrap().count(), 1);
                if separate_directory {
                    assert_eq!(
                        std::fs::read_dir(alias.parent().unwrap()).unwrap().count(),
                        1
                    );
                }
            }
        }
        for args in [&["list-components"][..], &["format"][..]] {
            let output = Command::new(env!("CARGO_BIN_EXE_m1-project"))
                .args(args)
                .arg("--project")
                .arg(&alias)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(alias.is_symlink());
            f.unchanged(&xml);
        }
    }
}

#[cfg(unix)]
#[test]
fn directory_project_symlinks_keep_xml_and_scripts_together() {
    use std::os::unix::fs::symlink;
    let xml = project(
        "<Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.F\" Filename=\"F.m1scr\"/>",
    );
    let f = Fixture::new(&xml);
    std::fs::create_dir(f.0.join("Scripts")).unwrap();
    std::fs::write(f.0.join("Scripts/F.m1scr"), b"// preserve code\n").unwrap();
    let alias = f.0.join("directory-alias");
    symlink(&f.0, &alias).unwrap();
    for args in [
        &["rename-component", "--name", "Root.F", "--new-name", "G"][..],
        &["create-function", "--name", "Root.New"][..],
        &["format", "--target", "10108"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_m1-project"))
            .args(args)
            .arg("--project")
            .arg(alias.join("Project.m1prj"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert!(alias.is_symlink());
    assert_eq!(
        std::fs::read(f.0.join("Scripts/G.m1scr")).unwrap(),
        b"// preserve code\n"
    );
    assert!(!f.0.join("Scripts/F.m1scr").exists());
    assert!(f.0.join("Scripts/New.m1scr").is_file());
    let out = std::fs::read_to_string(f.0.join("Project.m1prj")).unwrap();
    assert!(out.contains("Filename=\"G.m1scr\""));
    assert!(out.contains("FileFormat=\"10108\""));
}

#[test]
fn conversion_refuses_namespaced_return_metadata() {
    let returns = "<Returns><Return Type=\"f32\"><Description><![CDATA[KEEP]]></Description></Return></Returns>";
    for content in [
        returns.replace(
            "Type=\"f32\"",
            "Type=\"f32\" xmlns:a=\"urn:audit\" a:Type=\"KEEP_METADATA\"",
        ),
        returns.replace("<Return Type", "<Return Extra=\"KEEP_METADATA\" Type"),
        returns.replace("<Returns>", "<Returns xmlns=\"urn:audit\">"),
        returns
            .replace("<Return Type", "<a:Return xmlns:a=\"urn:audit\" Type")
            .replace("</Return>", "</a:Return>"),
        returns
            .replace("<Description>", "<a:Description xmlns:a=\"urn:audit\">")
            .replace("</Description>", "</a:Description>"),
        returns.replace(
            "<Description>",
            "<Description xmlns:a=\"urn:audit\" a:Extra=\"KEEP_METADATA\">",
        ),
        returns.replace("<Returns>", "<Returns xmlns:a=\"urn:audit\">"),
        returns.replace("Type=\"f32\"", "xmlns:a=\"urn:audit\" a:Type=\"f32\""),
        returns.replace("<Return Type", "<Return xml:space=\"preserve\" Type"),
    ] {
        let xml = project(&format!(
            "\n<Signature Name=\"\">\n {content}\n</Signature>\n"
        ));
        roxmltree::Document::parse(&xml).unwrap();
        let f = Fixture::new(&xml);
        for flag in [None, Some("--dry-run"), Some("--stdout")] {
            let mut args = vec!["format", "--target", "10108"];
            args.extend(flag);
            let output = f.run(&args);
            assert!(!output.status.success(), "{content}: {output:?}");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
            f.unchanged(&xml);
        }
        assert!(convert_format(&xml, 10108).is_err(), "{content}");
    }
    let malformed = project(
        "\n<Signature>\n <Returns><Return Type=\"f32\" a:Type=\"unbound\"/></Returns>\n</Signature>",
    );
    assert!(convert_format(&malformed, 10108).is_err());
    let f = Fixture::new(&malformed);
    assert!(!f.run(&["format", "--target", "10108"]).status.success());
    f.unchanged(&malformed);
}

#[test]
fn upgrade_refuses_namespaced_signature_content() {
    for content in [
        "<Signature Name=\"\" ReturnType=\"f32\">\n <Params/>\n <a:ReturnDescription xmlns:a=\"urn:audit\"><![CDATA[KEEP]]></a:ReturnDescription>\n</Signature>",
        "<Signature Name=\"\" ReturnType=\"\">\n <Params xmlns:a=\"urn:audit\" a:Extra=\"KEEP\"/>\n <ReturnDescription><![CDATA[]]></ReturnDescription>\n</Signature>",
        "<Signature xmlns=\"urn:audit\" Name=\"\" ReturnType=\"\">\n <Params/>\n <ReturnDescription><![CDATA[]]></ReturnDescription>\n</Signature>",
        "<Signature xmlns:a=\"urn:audit\" Name=\"\" a:ReturnType=\"KEEP\">\n <Params/>\n <ReturnDescription><![CDATA[]]></ReturnDescription>\n</Signature>",
        "<a:Signature xmlns:a=\"urn:audit\" ReturnType=\"\">\n <Params/>\n <ReturnDescription><![CDATA[]]></ReturnDescription>\n</a:Signature>",
    ] {
        let xml = project(&format!("\n{content}\n")).replace("10109", "10108");
        roxmltree::Document::parse(&xml).unwrap();
        let f = Fixture::new(&xml);
        let output = f.run(&["format", "--target", "10109"]);
        assert!(!output.status.success(), "{content}: {output:?}");
        f.unchanged(&xml);
        assert!(convert_format(&xml, 10109).is_err(), "{content}");
    }
}

#[test]
fn namespaced_signatures_remain_readable_and_noop_conversion_preserves_bytes() {
    for format in [10108, 10109] {
        let xml = project("\n<Signature xmlns:a=\"urn:audit\" a:ReturnType=\"KEEP\">\n <a:Returns/>\n</Signature>\n")
            .replace("10109", &format.to_string());
        assert_eq!(convert_format(&xml, format).unwrap(), xml);
        let f = Fixture::new(&xml);
        for args in [
            vec!["format"],
            vec![
                "format",
                "--target",
                if format == 10108 { "10108" } else { "10109" },
            ],
        ] {
            let output = f.run(&args);
            assert!(output.status.success(), "{output:?}");
            f.unchanged(&xml);
        }
    }
}

#[test]
fn later_signature_newline_error_preserves_the_entire_file() {
    for (from, to, return_type, body) in [
        (
            10108,
            10109,
            " ReturnType=\"f32\"",
            " <Params/>\n <ReturnDescription>\n<![CDATA[payload]]>\n </ReturnDescription>",
        ),
        (
            10109,
            10108,
            "",
            " <Returns>\n  <Return Type=\"f32\">\n   <Description>\n<![CDATA[payload]]>\n   </Description>\n  </Return>\n </Returns>",
        ),
    ] {
        let signature = format!("<Signature Name=\"\"{return_type}>\n{body}\n</Signature>");
        let good = signature.replace('\n', "\r\n");
        let mixed = good.replace("\r\n</Signature>", "\n</Signature>");
        let xml = project(&format!("{good}{mixed}"))
            .replace("FileFormat=\"10109\"", &format!("FileFormat=\"{from}\""));
        let fixture = Fixture::new(&xml);
        let output = fixture.run(&["format", "--target", &to.to_string()]);
        assert!(!output.status.success(), "from={from}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("newline"));
        fixture.unchanged(&xml);
    }
}
