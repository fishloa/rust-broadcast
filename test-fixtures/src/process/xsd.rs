//! `xmllint --schema` oracle for the ISO/IEC 23009-1 ed6 MPD schema.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{output_bounded, tool_available};

const SCHEMA_DIR_REL: &str = "specs/iso_iec_23009-1_dash_2026_ed6_schema";
const SCHEMA_FILE: &str = "DASH-MPD.xsd";
const XLINK_FILE: &str = "xlink.xsd";
const XLINK_REMOTE: &str = "http://www.w3.org/XML/2008/06/xlink.xsd";
const XLINK_STUB: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="http://www.w3.org/1999/xlink">
  <xs:attribute name="href" type="xs:anyURI"/>
  <xs:attribute name="actuate" type="xs:string"/>
  <xs:attribute name="type" type="xs:string"/>
  <xs:attribute name="show" type="xs:string"/>
</xs:schema>
"#;
const XMLLINT: &str = "xmllint";
const XMLLINT_VERSION_ARG: &str = "--version";
const DEADLINE: Duration = Duration::from_secs(60);

static COUNTER: AtomicU64 = AtomicU64::new(0);
static SCHEMA: OnceLock<PathBuf> = OnceLock::new();

fn offline_schema(private_dir: &Path) -> PathBuf {
    SCHEMA
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("dash-xsd-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create schema dir");
            for entry in std::fs::read_dir(private_dir)
                .expect("read schema dir")
                .flatten()
            {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "xsd") {
                    let text = std::fs::read_to_string(&path).expect("read xsd");
                    let text = text.replace(XLINK_REMOTE, XLINK_FILE);
                    std::fs::write(dir.join(entry.file_name()), text).expect("write xsd");
                }
            }
            std::fs::write(dir.join(XLINK_FILE), XLINK_STUB).expect("write xlink stub");
            dir.join(SCHEMA_FILE)
        })
        .clone()
}

/// The ed6 MPD schema and a usable `xmllint`, or `None` after a loud
/// `SKIPPED` line when either is absent.
#[must_use]
pub fn dash_mpd_schema(who: &str) -> Option<PathBuf> {
    if !tool_available(XMLLINT, XMLLINT_VERSION_ARG) {
        eprintln!("SKIPPED {who}: xmllint not on PATH");
        return None;
    }
    let dir = super::super::private_root()?.join(SCHEMA_DIR_REL);
    if !dir.join(SCHEMA_FILE).exists() {
        eprintln!("SKIPPED {who}: private schema not checked out");
        return None;
    }
    Some(offline_schema(&dir))
}

/// Validate `xml` against `schema`; the error carries xmllint's diagnostics.
///
/// # Errors
/// The xmllint stdout/stderr text when the document is rejected or the tool
/// cannot be run.
pub fn validate(schema: &Path, name: &str, xml: &str) -> Result<(), String> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("xsd_{}_{id}_{name}.xml", std::process::id()));
    std::fs::write(&tmp, xml).map_err(|e| e.to_string())?;
    let out = output_bounded(
        Command::new(XMLLINT)
            .arg("--noout")
            .arg("--schema")
            .arg(schema)
            .arg(&tmp),
        DEADLINE,
    );
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
