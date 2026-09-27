//! IMSC 1.1 profile validation — IMSC 1.1 §6–§9.
//!
//! Validation is separate from parsing: parse a document, then ask
//! "is this valid Text Profile?" or "is this valid Image Profile?"
//!
//! ### What is actually checked (#1108/TT-W3)
//!
//! This is a **partial** validator, not the full Feature/Extension
//! disposition table (159 rows, `imsc11-profiles.md` §5) its earlier module
//! doc claimed. `valid: true` means only that the checks below passed — it
//! is not a certification of full IMSC conformance. Implemented:
//!
//! - §7.9: the document claims the profile being validated against, via
//!   `ttp:contentProfiles`/`ttp:profile`.
//! - §7.12.4/§7.12.5: `ittp:aspectRatio`/`ttp:displayAspectRatio` mutual
//!   exclusion.
//! - §7.12.7 (IMSC 1.1 only): `ttp:frameRate` required when a frame term is
//!   used in a `body`/`div`/`p`/`img` `begin`/`dur`/`end` (not `span`).
//! - §7.12.1.3: at most 4 regions in `<layout>`.
//! - §8.4.11: `tts:textShadow` has at most 4 shadow values (Text Profile).
//! - §9.4.1: no `<p>` in an Image Profile `<div>` (`<span>`/`<br>` are not
//!   separately checked, since they can only appear inside a `<p>`).
//! - §12.3.1: every `body`/`div`/`p`/`img` `begin`/`dur`/`end` is a
//!   well-formed `<time-expression>` (via [`crate::time::parse_time_expression`]).
//!
//! **Not** checked (report the specific claim instead of assuming coverage
//! when using this validator): §7.12.1.2 region overlap/extent-past-RCR,
//! §7.12.6 extent-root, any `<region>`-level constraint (`validate_region`
//! is a stub — opacity/display/visibility/showBackground, §8.4.2/§9.4.2
//! `tts:extent` requirements), the Image Profile §9.4.4 `<image>` src/type/
//! extent requirements, §9.4.5 `smpte:backgroundImage`, and `<span>`-level
//! timing/frame-usage.
//!
//! ### Design
//!
//! The validator walks the parsed document tree and reports every
//! violation it finds, rather than stopping at the first error.
//! This gives callers a complete picture of non-conformance, for the
//! checks that are actually implemented.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::document::{self, *};
use crate::error::Error;

/// Which IMSC profile version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImscVersion {
    /// IMSC 1.0 / 1.0.1.
    V1_0,
    /// IMSC 1.1.
    V1_1,
}

impl ImscVersion {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            ImscVersion::V1_0 => "1.0",
            ImscVersion::V1_1 => "1.1",
        }
    }
}

broadcast_common::impl_spec_display!(ImscVersion);

/// Which IMSC profile to validate against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Profile {
    /// IMSC Text Profile.
    Text,
    /// IMSC Image Profile.
    Image,
}

impl Profile {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Profile::Text => "text",
            Profile::Image => "image",
        }
    }
}

broadcast_common::impl_spec_display!(Profile);

/// A single validation violation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ValidationError {
    /// The constraint that was violated (spec section reference).
    pub constraint: String,
    /// Additional detail about what was found.
    pub detail: String,
}

/// Accumulated validation results.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ValidationResult {
    /// Whether the document passed validation (no errors).
    pub valid: bool,
    /// All violations found.
    pub errors: Vec<ValidationError>,
}

/// Validator state: walks a parsed Document and accumulates violations.
#[derive(Debug, Clone)]
pub struct Validator {
    profile: Profile,
    version: ImscVersion,
    errors: Vec<ValidationError>,
}

impl Validator {
    /// Create a new validator for the given profile and version.
    pub fn new(profile: Profile, version: ImscVersion) -> Self {
        Self {
            profile,
            version,
            errors: Vec::new(),
        }
    }

    /// Validate a document against the configured profile.
    pub fn validate(mut self, doc: &Document) -> ValidationResult {
        self.validate_document(doc);
        ValidationResult {
            valid: self.errors.is_empty(),
            errors: self.errors,
        }
    }

    /// Convenience: validate and return `Result<(), Error>` with all violations
    /// concatenated into one error if any exist.
    pub fn validate_to_result(self, doc: &Document) -> Result<(), Error> {
        let result = self.validate(doc);
        if result.valid {
            Ok(())
        } else {
            let messages: Vec<String> = result
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.constraint, e.detail))
                .collect();
            Err(Error::Validation(messages.join("; ")))
        }
    }

    fn err(&mut self, constraint: &str, detail: String) {
        self.errors.push(ValidationError {
            constraint: constraint.to_string(),
            detail,
        });
    }

    fn validate_document(&mut self, doc: &Document) {
        // §7.1: Document Encoding — XML well-formedness is checked at parse time

        self.validate_tt(&doc.tt);

        if let Some(ref head) = doc.tt.head {
            self.validate_head(head);
        }
        if let Some(ref body) = doc.tt.body {
            self.validate_body(body);
            // §12.3.1: every begin/dur/end must actually be a well-formed
            // <time-expression> (#1108/TT-W3: this was never checked here —
            // `begin="garbage"` validated). Scoped to body/div/p/img, the
            // same set `body_has_frame_usage` already walks; span-level
            // timing is a documented gap (see the module doc).
            let ctx = doc.tt.time_context();
            self.validate_time_expr(body.begin.as_deref(), &ctx);
            self.validate_time_expr(body.dur.as_deref(), &ctx);
            self.validate_time_expr(body.end.as_deref(), &ctx);
            for div in &body.divs {
                self.validate_time_expr(div.begin.as_deref(), &ctx);
                self.validate_time_expr(div.dur.as_deref(), &ctx);
                self.validate_time_expr(div.end.as_deref(), &ctx);
                for p in &div.paragraphs {
                    self.validate_time_expr(p.begin.as_deref(), &ctx);
                    self.validate_time_expr(p.dur.as_deref(), &ctx);
                    self.validate_time_expr(p.end.as_deref(), &ctx);
                }
                for img in &div.images {
                    self.validate_time_expr(img.begin.as_deref(), &ctx);
                    self.validate_time_expr(img.dur.as_deref(), &ctx);
                    self.validate_time_expr(img.end.as_deref(), &ctx);
                }
            }
        }
    }

    fn validate_time_expr(&mut self, expr: Option<&str>, ctx: &crate::time::TimeContext) {
        if let Some(e) = expr
            && crate::time::parse_time_expression(e, ctx).is_err()
        {
            self.err(
                "IMSC §12.3.1",
                format!("'{e}' is not a well-formed time-expression"),
            );
        }
    }

    fn validate_tt(&mut self, tt: &TtElement) {
        // Check content profiles
        let claimed_text = self.claims_text_profile(tt);
        let claimed_image = self.claims_image_profile(tt);

        if self.profile == Profile::Text && !claimed_text {
            self.err(
                "IMSC §7.9",
                "Document does not claim Text Profile via ttp:contentProfiles or ttp:profile"
                    .into(),
            );
        }
        if self.profile == Profile::Image && !claimed_image {
            self.err(
                "IMSC §7.9",
                "Document does not claim Image Profile via ttp:contentProfiles or ttp:profile"
                    .into(),
            );
        }

        // §7.12.4 / §7.12.5: aspectRatio / displayAspectRatio mutual exclusion
        if tt.ittp_aspect_ratio.is_some() && tt.ttp_display_aspect_ratio.is_some() {
            self.err(
                "IMSC §7.12.4/§7.12.5",
                "ittp:aspectRatio and ttp:displayAspectRatio are mutually exclusive".into(),
            );
        }

        // §7.12.6 (extent-root) is NOT checked here — see the module doc.
        if self.version == ImscVersion::V1_1 {
            // §7.12.7: frameRate required if frame terms used
            // Check if any time expr uses 'f' metric or clock-time with frames
            if self.has_frame_usage(tt) && tt.ttp_frame_rate.is_none() {
                self.err(
                    "IMSC §7.12.7",
                    "ttp:frameRate must be present when frame terms are used".into(),
                );
            }
        }
        // Image Profile §9.4.1 is checked in `validate_div_image_constraints`;
        // §9.4.4 is NOT checked — see the module doc.
    }

    fn claims_text_profile(&self, tt: &TtElement) -> bool {
        let text_designators = [document::IMSC11_TEXT_PROFILE, document::IMSC1_TEXT_PROFILE];

        // Check ttp:contentProfiles
        if let Some(ref cp) = tt.ttp_content_profiles {
            for d in &text_designators {
                if cp.contains(d) {
                    return true;
                }
            }
        }

        // Check ttp:profile
        if let Some(ref p) = tt.ttp_profile {
            for d in &text_designators {
                if p == *d {
                    return true;
                }
            }
        }

        false
    }

    fn claims_image_profile(&self, tt: &TtElement) -> bool {
        let image_designators = [
            document::IMSC11_IMAGE_PROFILE,
            document::IMSC1_IMAGE_PROFILE,
        ];

        if let Some(ref cp) = tt.ttp_content_profiles {
            for d in &image_designators {
                if cp.contains(d) {
                    return true;
                }
            }
        }

        if let Some(ref p) = tt.ttp_profile {
            for d in &image_designators {
                if p == *d {
                    return true;
                }
            }
        }

        false
    }

    fn has_frame_usage(&self, tt: &TtElement) -> bool {
        // Scan the document's time expressions for frame metrics
        let body = match tt.body {
            Some(ref b) => b,
            None => return false,
        };
        Self::body_has_frame_usage(body)
    }

    fn body_has_frame_usage(body: &BodyElement) -> bool {
        // Checked once, outside the `div` loop (#1108/TT-W3): this used to
        // be inside `for div in &body.divs`, so a document with `<body
        // begin="...">` but no `<div>` at all never had `body`'s own
        // begin/dur/end inspected.
        if Self::time_expr_has_frame(body.begin.as_deref())
            || Self::time_expr_has_frame(body.dur.as_deref())
            || Self::time_expr_has_frame(body.end.as_deref())
        {
            return true;
        }
        for div in &body.divs {
            // `div`'s own timing (this loop previously covered only `p`/
            // `img`, never `div` itself).
            if Self::time_expr_has_frame(div.begin.as_deref())
                || Self::time_expr_has_frame(div.dur.as_deref())
                || Self::time_expr_has_frame(div.end.as_deref())
            {
                return true;
            }
            for p in &div.paragraphs {
                if Self::time_expr_has_frame(p.begin.as_deref())
                    || Self::time_expr_has_frame(p.dur.as_deref())
                    || Self::time_expr_has_frame(p.end.as_deref())
                {
                    return true;
                }
            }
            for img in &div.images {
                if Self::time_expr_has_frame(img.begin.as_deref())
                    || Self::time_expr_has_frame(img.dur.as_deref())
                    || Self::time_expr_has_frame(img.end.as_deref())
                {
                    return true;
                }
            }
        }
        false
    }

    fn time_expr_has_frame(expr: Option<&str>) -> bool {
        let expr = match expr {
            Some(e) => e,
            None => return false,
        };
        if expr.ends_with('f') && expr.len() > 1 {
            return true;
        }
        let colon_count = expr.chars().filter(|&c| c == ':').count();
        colon_count == 3
    }

    fn validate_head(&mut self, head: &HeadElement) {
        if let Some(ref layout) = head.layout {
            self.validate_layout(layout);
        }
    }

    fn validate_layout(&mut self, layout: &LayoutElement) {
        // §7.12.1.3: max 4 presented regions
        if layout.regions.len() > 4 {
            self.err(
                "IMSC §7.12.1.3",
                format!(
                    "Document has {} regions; maximum 4 presented regions allowed in any ISD",
                    layout.regions.len()
                ),
            );
        }

        // §7.12.1.2 (region overlap/extent-past-RCR) and §7.12.2/§7.12.3
        // (altText mutual exclusion) are NOT checked — see the module doc.
        for region in &layout.regions {
            self.validate_region(region);
        }
    }

    /// A stub: no per-region check is implemented yet (§7.12.1.1 presented
    /// region definition, §8.4.2/§9.4.2 `tts:extent` requirements) — see the
    /// module doc. Kept as a call site so a future check has somewhere to
    /// go without re-threading the region loop.
    fn validate_region(&mut self, _region: &RegionElement) {}

    fn validate_body(&mut self, body: &BodyElement) {
        // Image Profile §9.4.1: no p/span/br elements
        if self.profile == Profile::Image {
            for div in &body.divs {
                self.validate_div_image_constraints(div);
            }
        }

        // Text Profile §8.4.x constraints
        if self.profile == Profile::Text {
            for div in &body.divs {
                self.validate_div_text_constraints(div);
            }
        }
    }

    fn validate_div_image_constraints(&mut self, div: &DivElement) {
        // §9.4.1: p, span, br SHALL NOT be present
        if !div.paragraphs.is_empty() {
            self.err(
                "IMSC §9.4.1",
                format!(
                    "Image Profile div contains {} <p> element(s) — p/span/br SHALL NOT be present in Image Profile",
                    div.paragraphs.len()
                ),
            );
        }

        // §9.2.2, §9.4.4 and §9.4.5 are NOT checked — see the module doc.
    }

    fn validate_div_text_constraints(&mut self, div: &DivElement) {
        for p in &div.paragraphs {
            // §8.4.11: textShadow max 4 shadow values
            if let Some(ref ts) = p.style_attributes.tts_text_shadow {
                let count: usize = ts.split(',').count();
                if count > 4 {
                    self.err(
                        "IMSC §8.4.11",
                        format!("tts:textShadow has {} shadow values (max 4)", count),
                    );
                }
            }
        }
    }
}
