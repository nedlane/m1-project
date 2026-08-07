//! File-format detection (`format_report`) and lossless conversion
//! (`convert_format`) for a `Project.m1prj`.
//!
//! M1-Build silently upgrades a project's file format on open, with no prompt and
//! no supported way back; a newer M1-Build then locks every machine on an older
//! build out of the project. This module lets the toolchain (a) *see* the format a
//! project is at and which M1-Build wrote it, (b) convert between the known formats
//! — the downgrade is what unblocks a team member on an older build without
//! everyone upgrading in lockstep — and (c) gate the format in CI (via
//! `validate --max-format`, wired in `main`) so an accidental bump is caught in the
//! PR instead of on someone's laptop.
//!
//! The conversion is **surgical** and **byte-exact**, like every other edit here:
//! the only bytes that differ between `10108` and `10109` within a `.m1prj` are two
//! header attributes and the `<Signature>` elements, so only those are rewritten and
//! `10109 → 10108 → 10109` round-trips byte-for-byte (modulo the header attributes).
//!
//! The `10108 ↔ 10109` delta was derived by diffing the same real project before and
//! after M1-Build 1.4.5 migrated it (EV-M1, commits `267c2aa` ↔ `edd53ca`). It is
//! confined to `<Signature>`:
//!   - the `ReturnType="T"` attribute (10108) becomes `<Returns><Return Type="T">`
//!     (10109); a void `ReturnType=""` becomes nothing;
//!   - the `<ReturnDescription>CDATA</ReturnDescription>` block (10108) becomes the
//!     `<Return>`'s nested `<Description>CDATA</Description>` (10109); a void one is
//!     dropped;
//!   - an empty `<Params/>` (10108) is dropped in 10109 (non-empty `<Params>` is
//!     kept). The CDATA payload carries over unchanged in every case.
//!
//! Only the `10108 ↔ 10109` delta is known, so only that conversion is supported;
//! any other target is refused rather than guessed. See the incident report for the
//! unknowns still to resolve (validation against a differently-shaped project, the
//! full format→version table, whether an older build silently misreads a downgraded
//! file).

use crate::EditError;
use crate::xml::*;
use roxmltree::Node;
use std::ops::Range;

/// A known M1 project file format and the M1-Build point release(s) that write it.
/// Built up empirically — M1-Build does not document the mapping.
pub struct KnownFormat {
    /// The `FileFormat` attribute value on `<Project>`.
    pub file_format: u32,
    /// The canonical `ProductVersion` M1-Build stamps for this format, or `None`
    /// when we have no confirmed writer version (so conversion *to* it is refused).
    pub product_version: Option<&'static str>,
    /// Human description of the writer(s).
    pub writer: &'static str,
}

/// The formats seen in the wild so far. This is deliberately partial — even a
/// partial table beats the nothing that existed before. `10105` is attested in
/// older builds (EV-M1's `ProjectRevisions.db`) but its exact writer version and
/// format delta are unknown, so it is reported but not a conversion target.
pub const KNOWN_FORMATS: &[KnownFormat] = &[
    KnownFormat {
        file_format: 10105,
        product_version: None,
        writer: "M1 Build 1.4.x (earlier point release)",
    },
    KnownFormat {
        file_format: 10108,
        product_version: Some("1.4.4.981"),
        writer: "M1 Build 1.4.4.x",
    },
    KnownFormat {
        file_format: 10109,
        product_version: Some("1.4.5.556"),
        writer: "M1 Build 1.4.5.x",
    },
];

/// A one-line human summary of the known `format = writer` mappings, for the report.
pub fn known_writers_summary() -> String {
    KNOWN_FORMATS
        .iter()
        .map(|k| format!("{} = {}", k.file_format, k.writer))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The project's `FileFormat` (the `FileFormat` attribute on `<Project>`), if it
/// carries a numeric one. Used by `format`, and by `validate --max-format`.
pub fn file_format(xml: &str) -> Option<u32> {
    let doc = parse_xml(xml).ok()?;
    doc.descendants()
        .find(|n| n.has_tag_name("Project"))?
        .attribute("FileFormat")?
        .parse()
        .ok()
}

/// What `format` reports about a project's header, without converting anything.
#[derive(Debug, Clone)]
pub struct FormatReport {
    /// `FileFormat` on `<Project>`.
    pub file_format: Option<u32>,
    /// `ProductName` on `<MoTeCM1BuildSession>` (e.g. `M1Build (x64)`).
    pub product_name: Option<String>,
    /// `ProductVersion` on `<MoTeCM1BuildSession>` (e.g. `1.4.5.556`) — the build
    /// that last wrote the file.
    pub product_version: Option<String>,
    /// The `<System>` package target, formatted `Major.Minor.Release.Build`.
    pub package_target: Option<String>,
}

/// Parse a project's format header (read-only).
pub fn format_report(xml: &str) -> Result<FormatReport, EditError> {
    let doc = parse_xml(xml)?;
    let session = doc
        .descendants()
        .find(|n| n.has_tag_name("MoTeCM1BuildSession"));
    let project = doc.descendants().find(|n| n.has_tag_name("Project"));
    let system = doc.descendants().find(|n| n.has_tag_name("System"));
    Ok(FormatReport {
        file_format: project
            .and_then(|p| p.attribute("FileFormat"))
            .and_then(|s| s.parse().ok()),
        product_name: session
            .and_then(|s| s.attribute("ProductName"))
            .map(str::to_string),
        product_version: session
            .and_then(|s| s.attribute("ProductVersion"))
            .map(str::to_string),
        package_target: system.map(|s| {
            format!(
                "{}.{}.{}.{}",
                s.attribute("VersionMajor").unwrap_or("?"),
                s.attribute("VersionMinor").unwrap_or("?"),
                s.attribute("VersionRelease").unwrap_or("?"),
                s.attribute("VersionBuild").unwrap_or("?"),
            )
        }),
    })
}

/// Convert a project to the target `FileFormat`, returning the rewritten XML.
///
/// Byte-exact and reversible: only the two header attributes (`FileFormat`,
/// `ProductVersion`) and the `<Signature>` elements are touched, so
/// `10109 → 10108 → 10109` restores the original bytes (modulo those header
/// attributes). Incidental save artefacts M1-Build also changes on migration —
/// `<Project BuildNumber>`, the session `Locale` — are **not** touched; they are
/// not part of the format.
///
/// A no-op (returns the input) when the project is already at `target`. Refuses any
/// conversion other than `10108 ↔ 10109`, and refuses a project with no numeric
/// `FileFormat`.
pub fn convert_format(xml: &str, target: u32) -> Result<String, EditError> {
    let doc = parse_xml(xml)?;
    let project = doc
        .descendants()
        .find(|n| n.has_tag_name("Project"))
        .ok_or_else(|| EditError::Invalid("no <Project> element — not a project file".into()))?;
    let current: u32 = project
        .attribute("FileFormat")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            EditError::Invalid("project <Project> has no numeric FileFormat attribute".into())
        })?;

    if current == target {
        return Ok(xml.to_string());
    }
    if !matches!((current, target), (10108, 10109) | (10109, 10108)) {
        return Err(EditError::Invalid(format!(
            "unsupported conversion {current} -> {target}: only 10108 <-> 10109 is implemented \
             (the delta for other formats is not yet known)"
        )));
    }
    let upgrade = target == 10109;
    let product_version = KNOWN_FORMATS
        .iter()
        .find(|k| k.file_format == target)
        .and_then(|k| k.product_version)
        .expect("10108 and 10109 have canonical product versions");

    let mut fixes: Vec<(Range<usize>, String)> = Vec::new();

    // Header: FileFormat on <Project>, ProductVersion on <MoTeCM1BuildSession>.
    set_or_insert_attr(xml, project, "FileFormat", &target.to_string(), &mut fixes);
    if let Some(session) = doc
        .descendants()
        .find(|n| n.has_tag_name("MoTeCM1BuildSession"))
    {
        set_or_insert_attr(xml, session, "ProductVersion", product_version, &mut fixes);
    }

    // Signatures.
    for sig in doc.descendants().filter(|n| n.has_tag_name("Signature")) {
        collect_signature_fixes(xml, sig, upgrade, &mut fixes);
    }

    Ok(apply_splices_desc(xml.to_string(), fixes))
}

/// Push a fix that sets `attr="value"` on `node`, replacing the attribute if it is
/// present or inserting it just before the opening tag's `>` if it is not.
fn set_or_insert_attr(
    xml: &str,
    node: Node,
    attr: &str,
    value: &str,
    fixes: &mut Vec<(Range<usize>, String)>,
) {
    if let Some(a) = node.attribute_node(attr) {
        fixes.push((a.range(), format!("{attr}=\"{value}\"")));
    } else {
        let gt = open_tag_gt(xml, node);
        fixes.push((gt..gt, format!(" {attr}=\"{value}\"")));
    }
}

/// Byte offset of the `>` that closes `node`'s opening tag. Object/attribute names
/// and the values here never contain `>`, so the first `>` from the element start is
/// the tag close.
fn open_tag_gt(xml: &str, node: Node) -> usize {
    let start = node.range().start;
    start
        + xml[start..]
            .find('>')
            .expect("an element opening tag has a '>'")
}

/// The raw bytes inside `elem`'s single `<![CDATA[ … ]]>` section (empty if none).
/// Taken verbatim from the source so the payload round-trips byte-for-byte; CDATA
/// cannot contain `]]>`, so the delimiters are unambiguous.
fn cdata_payload<'a>(xml: &'a str, elem: Node) -> &'a str {
    let s = &xml[elem.range()];
    const OPEN: &str = "<![CDATA[";
    match (s.find(OPEN), s.rfind("]]>")) {
        (Some(a), Some(b)) if a + OPEN.len() <= b => &s[a + OPEN.len()..b],
        _ => "",
    }
}

/// Whether a `<Params>` element has no `<Param>` children (the empty `<Params/>`
/// that 10109 drops and 10108 requires).
fn params_is_empty(params: Node) -> bool {
    !params.children().any(|c| c.has_tag_name("Param"))
}

/// Collect the splices that convert one `<Signature>` between 10108 and 10109.
fn collect_signature_fixes(
    xml: &str,
    sig: Node,
    upgrade: bool,
    fixes: &mut Vec<(Range<usize>, String)>,
) {
    let b = indent_at(xml, sig.range().start).to_string();
    let c1 = format!("{b} ");
    let c2 = format!("{b}  ");
    let c3 = format!("{b}   ");

    let params = sig.children().find(|c| c.has_tag_name("Params"));
    let returns = sig.children().find(|c| c.has_tag_name("Returns"));
    let ret_desc = sig.children().find(|c| c.has_tag_name("ReturnDescription"));

    if upgrade {
        // 10108 -> 10109.
        // 1. Drop the ReturnType attribute (and the space that precedes it).
        if let Some(a) = sig.attribute_node("ReturnType") {
            let r = a.range();
            let start = if xml[..r.start].ends_with(' ') {
                r.start - 1
            } else {
                r.start
            };
            fixes.push((start..r.end, String::new()));
        }
        // 2. Drop an empty <Params/> (its whole line); keep a non-empty <Params>.
        if let Some(p) = params
            && params_is_empty(p)
        {
            let r = p.range();
            fixes.push((line_extended_start(xml, r.start)..r.end, String::new()));
        }
        // 3. <ReturnDescription> -> nested <Returns>/<Return>, or drop it if void.
        if let Some(rd) = ret_desc {
            let t = sig.attribute("ReturnType").unwrap_or("");
            let r = rd.range();
            if t.is_empty() {
                fixes.push((line_extended_start(xml, r.start)..r.end, String::new()));
            } else {
                let payload = cdata_payload(xml, rd);
                let block = format!(
                    "<Returns>\n{c2}<Return Type=\"{t}\">\n{c3}<Description>\n<![CDATA[{payload}]]>\n{c3}</Description>\n{c2}</Return>\n{c1}</Returns>"
                );
                fixes.push((r.start..r.end, block));
            }
        }
    } else {
        // 10109 -> 10108.
        let gt = open_tag_gt(xml, sig);
        // 1. Add the ReturnType attribute (T from <Return Type>, else "" for void).
        let t = returns
            .and_then(|r| r.children().find(|c| c.has_tag_name("Return")))
            .and_then(|r| r.attribute("Type"))
            .unwrap_or("");
        fixes.push((gt..gt, format!(" ReturnType=\"{t}\"")));
        // 2. Re-add an empty <Params/> as the first child when there is none.
        if params.is_none() {
            let after = gt + 1;
            fixes.push((after..after, format!("\n{c1}<Params/>")));
        }
        // 3. <Returns>/<Return> -> <ReturnDescription>, or append an empty one if void.
        if let Some(rn) = returns {
            let payload = rn
                .children()
                .find(|c| c.has_tag_name("Return"))
                .and_then(|r| r.children().find(|c| c.has_tag_name("Description")))
                .map(|d| cdata_payload(xml, d))
                .unwrap_or("");
            let r = rn.range();
            let block =
                format!("<ReturnDescription>\n<![CDATA[{payload}]]>\n{c1}</ReturnDescription>");
            fixes.push((r.start..r.end, block));
        } else {
            // Void: insert an empty <ReturnDescription> just before </Signature>.
            let sig_start = sig.range().start;
            let close = sig_start
                + xml[sig.range()]
                    .rfind("</Signature>")
                    .expect("signature has a closing tag");
            let nl = xml[..close]
                .rfind('\n')
                .expect("the closing tag sits on its own line");
            fixes.push((
                nl..nl,
                format!("\n{c1}<ReturnDescription>\n<![CDATA[]]>\n{c1}</ReturnDescription>"),
            ));
        }
    }
}
