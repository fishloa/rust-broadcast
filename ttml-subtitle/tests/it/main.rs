mod declaration_validation;
#[cfg(feature = "std")]
mod deep_drop;
#[cfg(feature = "std")]
mod error_variants;
mod golden;
#[cfg(feature = "std")]
mod hostile_structure;
mod integration;
mod limits;
#[cfg(feature = "std")]
mod mixed_content;
mod namespace_round_trip;
mod review_signalling_findings;
mod serialize_shape;
mod time_context;
mod time_expressions;
mod truncation;
#[cfg(feature = "std")]
mod typed_model;
#[cfg(feature = "std")]
mod unknown_attributes;
mod unknown_children_roundtrip;
mod validation_rules;
mod xml_declaration;
mod xml_escaping;
mod xml_space;
