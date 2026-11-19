//! Audit-22 #3041: `ObjectCarousel::write_xml` must emit its children when
//! *any* child list is populated, not only when all three are. The `no_children`
//! guard collapsing to `||` self-closes the element and silently drops the
//! carousel's whole content (the common "resource locators only" case included).
#![cfg(feature = "std")]

use dvb_mabr::{MulticastServerConfiguration, ObjectCarousel};

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";

fn parsed_document() -> MulticastServerConfiguration {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastServerConfiguration xmlns="{NS}" schemaVersion="2">
  <MulticastSession serviceIdentifier="urn:example:service">
    <PresentationManifestLocator manifestId="m1" contentType="application/dash+xml">https://example.com/m.mpd</PresentationManifestLocator>
    <MulticastTransportSession id="ts1" sessionIdleTimeout="1000">
      <TransportProtocol protocolIdentifier="urn:example:protocol" protocolVersion="1"/>
      <BitRate maximum="1000"/>
      <ObjectCarousel aggregateTransportSize="10">
        <PresentationManifests targetAcquisitionLatency="PT1S"/>
        <InitSegments targetAcquisitionLatency="PT2S"/>
        <ResourceLocator revalidationPeriod="PT1M">https://example.com/x.jpg</ResourceLocator>
      </ObjectCarousel>
    </MulticastTransportSession>
  </MulticastSession>
</MulticastServerConfiguration>"#
    );
    let parsed = MulticastServerConfiguration::parse_str(&xml).expect("parse base document");
    assert_eq!(
        parsed.sessions[0].transport_sessions[0]
            .object_carousel
            .as_ref()
            .expect("base carousel")
            .presentation_manifests
            .len(),
        1
    );
    parsed
}

fn base_carousel() -> ObjectCarousel {
    parsed_document().sessions[0].transport_sessions[0]
        .object_carousel
        .clone()
        .expect("base carousel")
}

fn round_trip_carousel(carousel: ObjectCarousel, child_markup: &str) {
    let mut doc = parsed_document();
    doc.sessions[0].transport_sessions[0].object_carousel = Some(carousel.clone());
    let out = doc.to_xml().expect("render");
    assert!(
        out.contains(child_markup),
        "the writer dropped the carousel's children ({child_markup} missing):\n{out}"
    );
    let reparsed = MulticastServerConfiguration::parse_str(&out).expect("re-parse");
    let got = reparsed.sessions[0].transport_sessions[0]
        .object_carousel
        .clone()
        .expect("carousel survives the round trip");
    assert_eq!(got, carousel, "round trip changed the carousel");
}

#[test]
fn a_carousel_with_only_presentation_manifests_keeps_them() {
    let mut carousel = base_carousel();
    carousel.init_segments.clear();
    carousel.resource_locators.clear();
    round_trip_carousel(carousel, "<PresentationManifests");
}

#[test]
fn a_carousel_with_only_init_segments_keeps_them() {
    let mut carousel = base_carousel();
    carousel.presentation_manifests.clear();
    carousel.resource_locators.clear();
    round_trip_carousel(carousel, "<InitSegments");
}

#[test]
fn a_carousel_with_only_resource_locators_keeps_them() {
    let mut carousel = base_carousel();
    carousel.presentation_manifests.clear();
    carousel.init_segments.clear();
    round_trip_carousel(carousel, "<ResourceLocator");
}

#[test]
fn an_empty_carousel_stays_self_closed() {
    let mut carousel = base_carousel();
    carousel.presentation_manifests.clear();
    carousel.init_segments.clear();
    carousel.resource_locators.clear();
    let mut doc = parsed_document();
    doc.sessions[0].transport_sessions[0].object_carousel = Some(carousel);
    let out = doc.to_xml().expect("render");
    assert!(
        out.contains("<ObjectCarousel aggregateTransportSize=\"10\"/>"),
        "an empty carousel must self-close:\n{out}"
    );
}
