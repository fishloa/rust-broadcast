mod declaration_validation;
#[cfg(feature = "std")]
mod deep_drop;
#[cfg(feature = "std")]
mod error_variants;
mod golden;
mod integration;
#[cfg(feature = "std")]
mod mixed_content;
mod review_signalling_findings;
mod truncation;
#[cfg(feature = "std")]
mod unknown_attributes;
mod unknown_children_roundtrip;
mod xml_declaration;
mod xml_escaping;
