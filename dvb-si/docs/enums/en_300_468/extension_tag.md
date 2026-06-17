# extension tag

_ETSI EN 300 468 Table 109 §6.4.0 — descriptor_tag_extension codes (ExtensionTag enum)_

> Values rendered from the co-located drift-guard [`extension_tag.toml`](./extension_tag.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `ImageIcon` | image_icon_descriptor |
| 0x04 | `T2DeliverySystem` | T2_delivery_system_descriptor |
| 0x05 | `ShDeliverySystem` | SH_delivery_system_descriptor |
| 0x06 | `SupplementaryAudio` | supplementary_audio_descriptor |
| 0x07 | `NetworkChangeNotify` | network_change_notify_descriptor |
| 0x08 | `Message` | message_descriptor |
| 0x09 | `TargetRegion` | target_region_descriptor |
| 0x0A | `TargetRegionName` | target_region_name_descriptor |
| 0x0B | `ServiceRelocated` | service_relocated_descriptor |
| 0x0D | `C2DeliverySystem` | C2_delivery_system_descriptor |
| 0x10 | `VideoDepthRange` | video_depth_range_descriptor |
| 0x11 | `T2mi` | T2-MI_descriptor |
| 0x13 | `UriLinkage` | URI_linkage_descriptor |
| 0x15 | `Ac4` | AC-4_descriptor |
| 0x16 | `C2BundleDeliverySystem` | C2_bundle_delivery_system_descriptor |
| 0x17 | `S2XSatelliteDeliverySystem` | S2X_satellite_delivery_system_descriptor |
| 0x19 | `AudioPreselection` | audio_preselection_descriptor |
| 0x20 | `TtmlSubtitling` | TTML_subtitling_descriptor |
| 0x22 | `ServiceProminence` | service_prominence_descriptor |
| 0x23 | `VvcSubpictures` | vvc_subpictures_descriptor |
| 0x24 | `S2Xv2SatelliteDeliverySystem` | S2Xv2_satellite_delivery_system_descriptor |
