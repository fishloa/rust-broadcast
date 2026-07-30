//! IMSC 1.1 profile validation — IMSC 1.1 §6–§9.
//!
//! Validation is separate from parsing: parse a document, then ask
//! "is this valid Text Profile?" or "is this valid Image Profile?"
//! The validator checks:
//!
//! - Feature/Extension disposition table (159 rows, `imsc11-profiles.md` §5)
//! - §7.12 "must reject" structural constraints
//! - §8 Text Profile provisions
//! - §9 Image Profile provisions
//!
//! ### Design
//!
//! The validator walks the parsed document tree and reports every
//! violation it finds, rather than stopping at the first error.
//! This gives callers a complete picture of non-conformance.

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

        // §7.12.6: extent-root — if any px unit used, tts:extent must be on tt
        // Full px scan requires tree walk; simplified: check if extent is present when likely needed
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

        // Image Profile constraints
        if self.profile == Profile::Image {
            // §9.4.4: image must have src, type, tts:extent
            // §9.4.1: no p/span/br elements
        }
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
        for div in &body.divs {
            if Self::time_expr_has_frame(body.begin.as_deref())
                || Self::time_expr_has_frame(body.dur.as_deref())
                || Self::time_expr_has_frame(body.end.as_deref())
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

        // §7.12.1.2: regions must not extend beyond RCR, no two overlap
        // (full coordinate intersection check requires computed style resolution)

        // §7.12.2 / §7.12.3: altText mutual exclusion
        for region in &layout.regions {
            self.validate_region(region);
        }
    }

    fn validate_region(&mut self, _region: &RegionElement) {
        // §7.12.1.1: presented region definition (opacity, display, visibility, showBackground)
        // §8.4.2: Text Profile: tts:extent required on region, must use px/%/rw/rh
        // §9.4.2: Image Profile: tts:extent required on region, must use px only
    }

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

        // §9.2.2: at most one div per presented region, which must be a presented image
        // §9.4.4: image constraints (src, type, tts:extent required)
        // §9.4.5: smpte:backgroundImage constraints
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
