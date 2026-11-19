mod declaration_validation;
#[cfg(feature = "std")]
mod deep_drop;
#[cfg(feature = "std")]
mod error_variants;
mod golden;
#[cfg(feature = "std")]
mod hostile_structure;
mod integration;
#[cfg(feature = "std")]
mod mixed_content;
mod review_signalling_findings;
mod time_context;
mod time_expressions;
mod truncation;
#[cfg(feature = "std")]
mod unknown_attributes;
mod unknown_children_roundtrip;
mod validation_rules;
mod xml_declaration;
mod xml_escaping;
