//! File-format conversion tests.
//!
//! The embedded tests exercise all four `<Signature>` shapes (typed/void ×
//! params/no-params) plus a text CDATA payload, asserting byte-exact output and a
//! byte-exact round trip. The `corpus_*` tests additionally verify the converter
//! against the real EV-M1 project before/after M1-Build 1.4.5 migrated it — set
//! `M1_FMT_10108` and `M1_FMT_10109` to the two file paths to run them; they skip
//! when unset (the real files are not committed).

use m1_project::convert_format;

/// Join `lines` with `\n` and a trailing `\n`. Written this way (rather than as one
/// backslash-continued string literal) because Rust's `\`-at-end-of-line strips the
/// next source line's leading whitespace — which would silently mangle the exact
/// indentation these byte-for-byte fixtures depend on.
fn blk(lines: &[&str]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// A minimal project wrapping one flat `<List>` of signature-bearing components.
/// `body` is spliced verbatim into the list so tests assert the exact MoTeC bytes.
fn wrap(file_format: u32, product_version: &str, body: &str) -> String {
    let head = blk(&[
        "<?xml version=\"1.0\"?>",
        &format!(
            "<MoTeCM1BuildSession ProductName=\"M1Build (x64)\" ProductVersion=\"{product_version}\">"
        ),
        &format!(" <Project FileFormat=\"{file_format}\" Name=\"T\">"),
        "  <System VersionMajor=\"1\" VersionMinor=\"4\" VersionRelease=\"0\" VersionBuild=\"0108\"/>",
        "  <ComponentStream>",
        "   <List>",
    ]);
    let tail = blk(&[
        "   </List>",
        "  </ComponentStream>",
        " </Project>",
        "</MoTeCM1BuildSession>",
    ]);
    format!("{head}{body}{tail}")
}

/// The four canonical signature shapes as real MoTeC writes them, at the `<List>`
/// component indentation (component at 4 spaces, `<Signature>` at 5). The CDATA
/// payload line is always flush-left, exactly as MoTeC writes it.
mod shapes {
    use super::blk;

    // Typed return, empty params.
    pub fn typed_10108() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncGenerated.Timer.Remaining\" Name=\"Root.A.Remaining\">",
            "     <Signature Name=\"\" ReturnType=\"f32\">",
            "      <Params/>",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <ReturnDescription>",
            "<![CDATA[]]>",
            "      </ReturnDescription>",
            "     </Signature>",
            "    </Component>",
        ])
    }
    pub fn typed_10109() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncGenerated.Timer.Remaining\" Name=\"Root.A.Remaining\">",
            "     <Signature Name=\"\">",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <Returns>",
            "       <Return Type=\"f32\">",
            "        <Description>",
            "<![CDATA[]]>",
            "        </Description>",
            "       </Return>",
            "      </Returns>",
            "     </Signature>",
            "    </Component>",
        ])
    }

    // Void return, empty params.
    pub fn void_empty_10108() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.A.Update\">",
            "     <Signature Name=\"\" ReturnType=\"\">",
            "      <Params/>",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <ReturnDescription>",
            "<![CDATA[]]>",
            "      </ReturnDescription>",
            "     </Signature>",
            "    </Component>",
        ])
    }
    pub fn void_empty_10109() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.A.Update\">",
            "     <Signature Name=\"\">",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "     </Signature>",
            "    </Component>",
        ])
    }

    // Void return, NON-empty params (params kept in both formats).
    pub fn void_params_10108() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncGenerated.Timer.Set\" Name=\"Root.A.Start\">",
            "     <Signature Name=\"\" ReturnType=\"\">",
            "      <Params>",
            "       <Param Name=\"Timeout\" Type=\"f32\" Attrs=\"0\">",
            "        <Description>",
            "<![CDATA[]]>",
            "        </Description>",
            "       </Param>",
            "      </Params>",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <ReturnDescription>",
            "<![CDATA[]]>",
            "      </ReturnDescription>",
            "     </Signature>",
            "    </Component>",
        ])
    }
    pub fn void_params_10109() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncGenerated.Timer.Set\" Name=\"Root.A.Start\">",
            "     <Signature Name=\"\">",
            "      <Params>",
            "       <Param Name=\"Timeout\" Type=\"f32\" Attrs=\"0\">",
            "        <Description>",
            "<![CDATA[]]>",
            "        </Description>",
            "       </Param>",
            "      </Params>",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "     </Signature>",
            "    </Component>",
        ])
    }

    // Typed return carrying a non-empty CDATA payload (must carry over verbatim).
    pub fn payload_10108() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.A.Calc\">",
            "     <Signature Name=\"\" ReturnType=\"bool\">",
            "      <Params/>",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <ReturnDescription>",
            "<![CDATA[true when armed]]>",
            "      </ReturnDescription>",
            "     </Signature>",
            "    </Component>",
        ])
    }
    pub fn payload_10109() -> String {
        blk(&[
            "    <Component Classname=\"BuiltIn.FuncUserParam\" Name=\"Root.A.Calc\">",
            "     <Signature Name=\"\">",
            "      <Description>",
            "<![CDATA[]]>",
            "      </Description>",
            "      <DescriptionFull>",
            "<![CDATA[]]>",
            "      </DescriptionFull>",
            "      <Returns>",
            "       <Return Type=\"bool\">",
            "        <Description>",
            "<![CDATA[true when armed]]>",
            "        </Description>",
            "       </Return>",
            "      </Returns>",
            "     </Signature>",
            "    </Component>",
        ])
    }
}

fn assert_pair(body_10108: &str, body_10109: &str) {
    let p8 = wrap(10108, "1.4.4.981", body_10108);
    let p9 = wrap(10109, "1.4.5.556", body_10109);

    // Upgrade produces exactly the 10109 bytes, downgrade exactly the 10108 bytes.
    assert_eq!(
        convert_format(&p8, 10109).unwrap(),
        p9,
        "upgrade 10108 -> 10109 mismatch"
    );
    assert_eq!(
        convert_format(&p9, 10108).unwrap(),
        p8,
        "downgrade 10109 -> 10108 mismatch"
    );
    // Round trips are byte-identical.
    assert_eq!(
        convert_format(&convert_format(&p8, 10109).unwrap(), 10108).unwrap(),
        p8,
        "10108 -> 10109 -> 10108 not byte-identical"
    );
    assert_eq!(
        convert_format(&convert_format(&p9, 10108).unwrap(), 10109).unwrap(),
        p9,
        "10109 -> 10108 -> 10109 not byte-identical"
    );
}

#[test]
fn typed_return_empty_params() {
    assert_pair(&shapes::typed_10108(), &shapes::typed_10109());
}

#[test]
fn void_return_empty_params() {
    assert_pair(&shapes::void_empty_10108(), &shapes::void_empty_10109());
}

#[test]
fn void_return_non_empty_params() {
    assert_pair(&shapes::void_params_10108(), &shapes::void_params_10109());
}

#[test]
fn typed_return_carries_cdata_payload() {
    assert_pair(&shapes::payload_10108(), &shapes::payload_10109());
}

#[test]
fn all_shapes_together() {
    // Several signatures in one file — exercises the multi-splice ordering.
    let body8 = format!(
        "{}{}{}{}",
        shapes::typed_10108(),
        shapes::void_empty_10108(),
        shapes::void_params_10108(),
        shapes::payload_10108()
    );
    let body9 = format!(
        "{}{}{}{}",
        shapes::typed_10109(),
        shapes::void_empty_10109(),
        shapes::void_params_10109(),
        shapes::payload_10109()
    );
    assert_pair(&body8, &body9);
}

#[test]
fn same_format_is_a_noop() {
    let p8 = wrap(10108, "1.4.4.981", &shapes::typed_10108());
    assert_eq!(convert_format(&p8, 10108).unwrap(), p8);
}

#[test]
fn unsupported_conversion_is_refused() {
    let p8 = wrap(10108, "1.4.4.981", &shapes::void_empty_10108());
    let err = convert_format(&p8, 10105).unwrap_err().to_string();
    assert!(err.contains("unsupported conversion"), "got: {err}");
}

#[test]
fn header_is_rewritten_on_convert() {
    let p8 = wrap(10108, "1.4.4.981", &shapes::void_empty_10108());
    let up = convert_format(&p8, 10109).unwrap();
    assert!(up.contains("FileFormat=\"10109\""));
    assert!(up.contains("ProductVersion=\"1.4.5.556\""));
    assert!(!up.contains("FileFormat=\"10108\""));
}

// ---- Real-corpus tests (skip unless the two fixture paths are provided) ----

fn corpus_pair() -> Option<(String, String)> {
    let p8 = std::env::var("M1_FMT_10108").ok()?;
    let p9 = std::env::var("M1_FMT_10109").ok()?;
    Some((
        std::fs::read_to_string(p8).expect("read 10108 fixture"),
        std::fs::read_to_string(p9).expect("read 10109 fixture"),
    ))
}

/// Every `<Signature>...</Signature>` block, in document order.
fn signature_blocks(xml: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find("<Signature ") {
        let after = &rest[a..];
        let end = after.find("</Signature>").unwrap() + "</Signature>".len();
        out.push(&after[..end]);
        rest = &after[end..];
    }
    out
}

#[test]
fn corpus_upgrade_matches_real_migration() {
    let Some((real8, real9)) = corpus_pair() else {
        eprintln!("skipping: set M1_FMT_10108 / M1_FMT_10109 to run the corpus test");
        return;
    };
    let up = convert_format(&real8, 10109).unwrap();
    // The signature bodies our upgrade produces must match M1-Build's real 10109
    // output exactly (the whole file differs only in the incidental BuildNumber /
    // Locale that we deliberately leave alone).
    assert_eq!(
        signature_blocks(&up),
        signature_blocks(&real9),
        "upgraded signatures do not match M1-Build's real 10109 output"
    );
}

#[test]
fn corpus_round_trip_is_byte_identical() {
    let Some((real8, real9)) = corpus_pair() else {
        eprintln!("skipping: set M1_FMT_10108 / M1_FMT_10109 to run the corpus test");
        return;
    };
    assert_eq!(
        convert_format(&convert_format(&real8, 10109).unwrap(), 10108).unwrap(),
        real8,
        "10108 -> 10109 -> 10108 not byte-identical on the real project"
    );
    assert_eq!(
        convert_format(&convert_format(&real9, 10108).unwrap(), 10109).unwrap(),
        real9,
        "10109 -> 10108 -> 10109 not byte-identical on the real project"
    );
}
