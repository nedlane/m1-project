use crate::{Finding, FindingLevel};
use std::path::PathBuf;

/// Selected module definitions and explicit warnings for unavailable metadata.
/// No unselected files are read or inferred from component class names.
pub struct SelectedModules {
    /// Tolerantly decoded XML for the exact selected module filenames.
    pub xmls: Vec<String>,
    /// Coverage warnings for selected definitions that could not be found.
    pub findings: Vec<Finding>,
}

/// Read the exact module-set versions selected by `project_xml`.
///
/// Explicit directories take precedence over `M1_MODULES_PATH`; without them,
/// the environment and standard Windows/WSL M1 Build module locations are used.
/// Missing, unreadable or invalid module definitions produce coverage warnings
/// and are skipped without discarding the parseable project model. Malformed
/// project XML produces an error. Filenames normalize zero-padded version fields and no
/// fallback to a different version is attempted. Names must be single safe
/// filename components and all selected version fields must be decimal digits.
/// Directory order is preserved, so the first matching definition wins.
pub fn selected_module_xmls(
    project_xml: &str,
    explicit_dirs: &[PathBuf],
) -> Result<SelectedModules, Box<dyn std::error::Error>> {
    let doc = roxmltree::Document::parse(project_xml)?;
    let mut selected = Vec::new();
    let mut findings = Vec::new();
    'selection: for file in doc
        .descendants()
        .find(|node| node.has_tag_name("SelectedModuleSets"))
        .into_iter()
        .flat_map(|sets| sets.children().filter(|node| node.has_tag_name("File")))
    {
        let Some(name) = file.attribute("Name") else {
            findings.push(incomplete("selected module is missing Name"));
            continue;
        };
        if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', ':']) {
            findings.push(incomplete(
                "selected module Name must be a single safe filename component",
            ));
            continue;
        }
        let attributes = ["VersionMajor", "VersionMinor", "VersionBuild"];
        if attributes
            .iter()
            .any(|attribute| file.attribute(*attribute).is_none())
        {
            findings.push(Finding {
                level: FindingLevel::Warning,
                path: "SelectedModuleSets".into(),
                message: format!("selected module `{name}` has incomplete version metadata; inherited tag and generated component checks are incomplete. Library export ownership validation also remains limited"),
                code: None,
            });
            continue;
        }
        let mut version = Vec::new();
        for attribute in attributes {
            let value = file
                .attribute(attribute)
                .expect("version presence checked above");
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                findings.push(incomplete(format!(
                    "selected module `{name}` {attribute} must contain only digits"
                )));
                continue 'selection;
            }
            version.push(normalise_version_part(value));
        }
        let filename = format!("{name}.{}.{}.{}.m1mod", version[0], version[1], version[2]);
        selected.push((filename, name, version));
    }

    let search_dirs = module_search_dirs(explicit_dirs);
    let mut xmls = Vec::new();
    for (filename, selected_name, selected_version) in selected {
        let Some(path) = search_dirs
            .iter()
            .map(|dir| dir.join(&filename))
            .find(|path| path.is_file())
        else {
            findings.push(Finding {
                level: FindingLevel::Warning,
                path: "SelectedModuleSets".into(),
                message: format!(
                    "selected module metadata `{filename}` was not found; inherited tag and generated component checks are incomplete for this module. Supply metadata with --modules-dir or M1_MODULES_PATH. Library export ownership validation also remains limited"
                ),
                code: None,
            });
            continue;
        };
        let loaded = (|| -> Result<String, Box<dyn std::error::Error>> {
            let xml = m1_workspace::read_text(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let module_doc = roxmltree::Document::parse(&xml)
                .map_err(|error| format!("{}: invalid .m1mod XML: {error}", path.display()))?;
            let root = module_doc.root_element();
            if !root.has_tag_name("MoTecM1BuildModuleSet")
                || root.attribute("Name") != Some(selected_name)
            {
                return Err(format!(
                    "{}: module metadata identity does not match selected module `{selected_name}`",
                    path.display()
                )
                .into());
            }
            // Some exported module sets omit version attributes. Their exact selected
            // filename remains authoritative; reject mismatches when XML declares one.
            for (attribute, selected_part) in ["VersionMajor", "VersionMinor", "VersionBuild"]
                .iter()
                .zip(selected_version)
            {
                if let Some(actual) = root.attribute(*attribute)
                    && (actual.is_empty()
                        || !actual.bytes().all(|byte| byte.is_ascii_digit())
                        || normalise_version_part(actual) != selected_part)
                {
                    return Err(format!(
                        "{}: module metadata {attribute} does not match the selected version",
                        path.display()
                    )
                    .into());
                }
            }
            Ok(xml)
        })();
        match loaded {
            Ok(xml) => xmls.push(xml),
            Err(error) => findings.push(Finding {
                level: FindingLevel::Warning,
                path: "SelectedModuleSets".into(),
                message: format!("{error}; inherited tag and generated component checks are incomplete for this module. Library export ownership validation also remains limited"),
                code: None,
            }),
        }
    }
    Ok(SelectedModules { xmls, findings })
}

fn incomplete(reason: impl std::fmt::Display) -> Finding {
    Finding {
        level: FindingLevel::Warning,
        path: "SelectedModuleSets".into(),
        message: format!(
            "{reason}; inherited tag and generated component checks are incomplete for this module. Library export ownership validation also remains limited"
        ),
        code: None,
    }
}

fn normalise_version_part(part: &str) -> &str {
    let trimmed = part.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn module_search_dirs(explicit_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = if explicit_dirs.is_empty() {
        std::env::var_os("M1_MODULES_PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default()
    } else {
        explicit_dirs.to_vec()
    };
    if explicit_dirs.is_empty() {
        if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
            dirs.push(
                PathBuf::from(program_data)
                    .join("MoTeC")
                    .join("M1")
                    .join("Build")
                    .join("Modules"),
            );
        }
        // WSL can run the Linux build while sharing the host M1-Build install.
        dirs.push(PathBuf::from("/mnt/c/ProgramData/MoTeC/M1/Build/Modules"));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_rejects_path_components_and_non_numeric_versions() {
        for name in ["../Outside", "..\\Outside", "/Outside", "C:Outside", ".."] {
            let xml = format!(
                r#"<Project><SelectedModuleSets><File Name="{name}" VersionMajor="1" VersionMinor="2" VersionBuild="3"/></SelectedModuleSets></Project>"#
            );
            let result = selected_module_xmls(&xml, &[]).unwrap();
            assert!(result.xmls.is_empty(), "{name}");
            assert_eq!(result.findings.len(), 1);
        }
        for version in ["../3", "3/Outside", "3\\Outside", "", "-3", "3x"] {
            let xml = format!(
                r#"<Project><SelectedModuleSets><File Name="Test Module" VersionMajor="1" VersionMinor="2" VersionBuild="{version}"/></SelectedModuleSets></Project>"#
            );
            let result = selected_module_xmls(&xml, &[]).unwrap();
            assert!(result.xmls.is_empty(), "{version}");
            assert_eq!(result.findings.len(), 1);
        }
    }

    #[test]
    fn explicit_search_order_is_preserved() {
        let dirs = vec![
            PathBuf::from("z-first"),
            PathBuf::from("a-second"),
            PathBuf::from("z-first"),
        ];
        assert_eq!(module_search_dirs(&dirs), dirs[..2]);
    }

    #[test]
    fn discovery_uses_exact_selected_version_and_checks_declared_identity() {
        let dir = std::env::temp_dir().join(format!("m1_selected_modules_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = r#"<MoTeCM1BuildSession><Project><SelectedModuleSets><File Name="Test Module" VersionMajor="01" VersionMinor="02" VersionBuild="0003"/></SelectedModuleSets></Project></MoTeCM1BuildSession>"#;
        // An adjacent version must not satisfy the project's selection.
        std::fs::write(
            dir.join("Test Module.1.2.4.m1mod"),
            r#"<MoTecM1BuildModuleSet Name="Test Module"/>"#,
        )
        .unwrap();
        let missing = selected_module_xmls(project, std::slice::from_ref(&dir)).unwrap();
        assert!(missing.xmls.is_empty());
        assert_eq!(missing.findings.len(), 1);
        let selected = dir.join("Test Module.1.2.3.m1mod");
        std::fs::write(&selected, r#"<MoTecM1BuildModuleSet Name="Wrong Module"/>"#).unwrap();
        assert!(
            selected_module_xmls(project, std::slice::from_ref(&dir))
                .unwrap()
                .xmls
                .is_empty()
        );
        std::fs::write(
            &selected,
            r#"<MoTecM1BuildModuleSet Name="Test Module" VersionBuild="4"/>"#,
        )
        .unwrap();
        assert!(
            selected_module_xmls(project, std::slice::from_ref(&dir))
                .unwrap()
                .xmls
                .is_empty()
        );
        std::fs::write(&selected, "<broken>").unwrap();
        let malformed = selected_module_xmls(project, std::slice::from_ref(&dir))
            .unwrap()
            .findings
            .remove(0)
            .message;
        assert!(malformed.contains(selected.to_str().unwrap()));
        assert!(malformed.contains("invalid .m1mod XML"));
        std::fs::write(
            &selected,
            r#"<MoTecM1BuildModuleSet Name="Test Module" VersionBuild=""/>"#,
        )
        .unwrap();
        assert!(
            selected_module_xmls(project, std::slice::from_ref(&dir))
                .unwrap()
                .xmls
                .is_empty()
        );
        // An empty declared field must not normalize to the selected zero.
        let zero_project = project.replace("VersionBuild=\"0003\"", "VersionBuild=\"0\"");
        std::fs::write(
            dir.join("Test Module.1.2.0.m1mod"),
            r#"<MoTecM1BuildModuleSet Name="Test Module" VersionBuild=""/>"#,
        )
        .unwrap();
        let empty_version =
            selected_module_xmls(&zero_project, std::slice::from_ref(&dir)).unwrap();
        assert!(empty_version.xmls.is_empty());
        assert_eq!(empty_version.findings.len(), 1);
        std::fs::write(&selected, r#"<MoTecM1BuildModuleSet Name="Test Module" VersionMajor="1" VersionMinor="2" VersionBuild="3"/>"#).unwrap();
        let matched = selected_module_xmls(project, std::slice::from_ref(&dir)).unwrap();
        assert_eq!(matched.xmls.len(), 1);
        assert!(matched.findings.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
