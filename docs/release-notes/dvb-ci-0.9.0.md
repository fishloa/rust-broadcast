# dvb-ci 0.9.0

_Released 2026-10-05._

Breaking release (0.8 -> 0.9) for the DVB Common Interface (EN 50221, CI Plus, TS 103 205 / TS 101 699 extensions) codecs. It is driven by the #1091 audit and has three themes: parse and serialize now agree (fixed-layout APDUs reject trailing bytes, serializers reject values that do not fit their wire field instead of truncating them, reserved or inconsistent combinations that `parse` could not read back are refused), a few panicking paths become fallible, and secrets no longer appear in `Debug` output. It also moves to the `dvb-si` 11 line. You must act if you call `tpdu::tc_object_bytes`, use `LscV4Apdu` / `LscV4ReplyApdu` / `parse_ip_config_reply`, construct `ActivationStatus`, or call `SacMessage::to_bytes`; and anything that talks to a CAM should expect stricter input handling (see Behaviour changes). The stricter parsers are not verified against real hardware.

Use together with `dvb-ci-runtime-0.17.0.md` (which depends on this release) and `dvb-si-11.0.0.md`.

## Breaking changes

1. **`tpdu::tc_object_bytes` returns `Result<Vec<u8>>`** (was `Vec<u8>`; it panicked through `to_bytes` for a `TcObject` with a non-connection tag). `TcObject::parse` and its serializer now reject any `tpdu_tag` other than the five single-`t_c_id` connection objects (Create, C_T_C_Reply, Delete, D_T_C_Reply, Request) instead of accepting and re-emitting any byte (audit r10-O-3, #1091). The new `TcObject::is_tc_tag(tag: u8) -> bool` tells you whether a tag is one of them.
   ```rust
   // before: let bytes = tc_object_bytes(&obj);
   // after:  let bytes = tc_object_bytes(&obj)?;
   ```
2. **`ci_plus::low_speed_comms_v4::LscV4Apdu` dispatches all four LSC v4 APDUs**, including `comms_IP_config_reply`, and is no longer `Copy` (the reply carries a `Vec` of DNS servers). `LscV4ReplyApdu` and `parse_ip_config_reply` are removed; use `LscV4Apdu::parse` or `CommsIpConfigReply::parse`. The old doc promised an `Ok(None)` that the code never returned (audit r10-O-7).
3. **`ci_ext::status_query::ActivationStatus`**: `activation_state` is now `ActivationState`, not `Option<ActivationState>` (`parse` always produced `Some`, so a serialized `None` could never round-trip); the struct gains `reserved: u8` (the 4 reserved bits `[7:4]`, so all 256 byte values round-trip) and no longer derives `Default` (audit r10-O-8).
   ```rust
   // before: ActivationStatus { event_activated, activation_state: Some(s) }
   // after:  ActivationStatus { event_activated, activation_state: s, reserved: 0 }
   ```
4. **`ci_plus::content_control::SacMessage::to_bytes` is removed** (it panicked for a hand-built value with a datatype value over 64 KiB). Use `SacMessage::try_to_bytes() -> Result<Vec<u8>>` (audit r10-W-14, #1091).
5. **Serializers return errors instead of truncating** when a length, count or PID does not fit its wire field (#1129). The affected fields are listed under Fixes; a value you could previously serialize "successfully" may now be `Err`. Separately, one serializer's output bytes change: `objects::mmi_display::DisplayReply` writes the `pixel_depth` reserved bits as `1`s (was `00`) (audit r10-O-2).

## Behaviour changes (no signature change)

- **Fixed-layout APDUs reject trailing bytes** (audit r10-O-1, #1091). An APDU whose `length_field` declares bytes past its fixed body now fails with `Error::InvalidObject` ("trailing bytes after the fixed body") instead of dropping the extras, which broke parse -> serialize. The EN 50221, CI+ and TS 101 699 `length_field` covers exactly the listed fields in every one of these tables (`docs/en_50221/`, `docs/ci_plus/`, `docs/ts_103_205/`). The strict parsers are: `host_control::{Tune, Replace}`; `low_speed_comms::{CommsReply, CommsCmd (connect, set_params, get_next_buffer and every parameterless command)}`; `mmi_close::CloseMmi`; `mmi_display::{DisplayControl, DownloadReply}`; `mmi_high::Answ` (cancel); `resource_manager_v2::{ModuleIdSend, ModuleIdCommand}`; `application_mmi::RequestStartAck`; `copy_protection::CpReply`; `event_manager::{EventRequestAck, EventNotification}`; `power_manager::{ActivationStateChangeRequest, ActivationStateChangeAck}`; `service_gateway::GetServiceAck`; `broadcast_service_gateway::EitSectionReq`; `stream_input::{ScanAck, TuneTSAck}`; `sample_decryption::{SdStartReply, SdUpdateReply}`; `multistream::CicamMultistreamCapability`; `cicam_player::{PlayerVerifyReply, PlayerStartReply, PlayerStatusError, PlayerInfoReply, PlayerAssetEnd, PlayerUpdateReply, PlayerCapabilitiesReply, PlayerControlReq}`; `low_speed_comms_v4::CommsInfoReply`; `file_retrieval::FileSystemAck`; `content_control::{CcPinReply, CcPinEvent}`; `multistream_host_control::{TuneTripletReq, TuneLcnReq}`. Variable-length bodies (lists, text, opaque blobs) are unchanged. A CAM that pads these APDUs is now refused, and `dvb-ci-runtime` reports it as `Notification::Error`. Pinned by one table-driven test (`strict_tests`); not verified against real hardware.
- **Over-wide values are rejected instead of masked:** `ci_ext::{copy_protection, software_download}` reject a `CopyProtectionID` / `BinaryId.specifier` over 24 bits (the top byte used to be dropped); `ci_plus::multistream_host_control` rejects a `logical_channel_number` over 14 bits and `background_tune = true` in a `BaseV3` `TuneIpReq`.
- **`cicam_player::ControlCommand::Reserved(0x01 | 0x02)`** no longer serializes a parameterless SetPosition/SetSpeed byte, and a reserved command that arrives with a payload is rejected rather than truncated.
- **`length::decode` no longer accepts a `length_field_size` of 3.** `encode_into` caps values at `0xFFFF` (two length bytes), so a decoded size of 3 could never be reproduced. Non-minimal long forms (for example `5` sent in the 2-byte long form) are still accepted, because real hardware sends them (`dvb-ci-runtime`'s `decodes_long_form_length_profile_reply`, #337).
- **Secrets are redacted in `Debug`** (audit #1142). `SacDatatype` redacts the value of the PIN, licence and every unnamed (key-exchange) datatype; `CcPinEvent` its `private_data`; `DrmMetadataRecord` its `drm_metadata` blob; `Answ` the typed answer text (a blind enquiry's PIN). Only the length is shown. The fields themselves and `serde` output are unchanged.

## New API

- `builder::CaPmtBuilt::try_to_bytes() -> Result<Vec<u8>>`: the fallible counterpart of `to_bytes`. It returns `Error::InvalidObject` when a filtered descriptor loop has no valid `ca_pmt` encoding (for example a corrupt source PMT whose projected `program_info_length` exceeds its 12-bit field), so a host forwarding a CAM-supplied PMT can reject it instead of panicking.
- `TcObject::is_tc_tag(tag: u8) -> bool`, and an associated `TAGS: &'static [ApduTag]` on each of the 18 resource-scoped `*Apdu` dispatch enums (generated by the new internal `declare_resource_apdus!`, which rejects a tag declared twice at compile time) (audit r10-O-4, #1091).

## Fixes

Truncating serializers, now range-checked (#1091, #1129); each previously returned `Ok` with wrapped bytes:
- `objects::ca_pmt::CaPmt::serialize_into`: `version_number` (5-bit), `program_info_length` / `ES_info_length` (12-bit), `elementary_pid` (13-bit), now `Error::InvalidObject` (the host-to-CAM direction that the earlier #972 PID fix missed). `CaPmtReply::serialize_into`: `version_number`.
- `objects::mmi_display::DisplayReply`: `aspect_ratio_information` (4-bit), `graphics_relation_to_video` (3-bit), `display_bytes` (12-bit), `number_pixel_depths` (4-bit), a pixel-depth entry's `display_depth` / `pixels_per_byte` (3-bit each).
- `ci_plus::ca_support` (`MsCaPmt` / `MsCaPmtReply`) had inherited the truncation bugs from verbatim copies of the `ca_pmt` helpers and lacked the #972 PID fix. The four helpers (`info_block_len`, `parse_cmd_and_descriptors`, `write_info_block`, `encode_enable_byte`) are now `pub(crate)` and shared, and `PMT_PID`, `version_number`, `program_info_length` / `ES_info_length`, `elementary_pid` are range-checked.
- `ci_plus::multistream::{PidSelectReq, PidSelectReply}`: `num_PID` (8-bit), PID entries (13-bit).
- `ci_plus::sample_decryption`: `number_of_DRM_system_ids` / `number_of_DRM_UUIDs`, `number_of_metadata_records` / `number_of_Sample_Tracks` (8-bit), `track_PID` (13-bit), `drm_metadata_length` (16-bit).
- `ci_plus::multistream_host_control`: `service_location_length` (12-bit), `num_dsd` (7-bit). `ci_plus::low_speed_comms_v4`: `inputDeliveryPID` (13-bit), `num_DNS_servers` (8-bit). `ci_plus::file_retrieval::FileSystemOffer`: `DomainIdentifierLength` (8-bit).
- `ci_ext::software_download` DSM-CC messages (`DownloadInfoRequest`, `DownloadInfoResponse`, `DownloadCancel`, `DownloadDataRequest`, `DownloadDataBlock`): `adaptationLength` (8-bit), `messageLength` and `privateDataLength` (16-bit). A `DownloadDataBlock` with a 64 KiB or larger block used to get a wrapped `messageLength` and `Ok`.
- `ci_ext::service_gateway::ServiceDescAck` / `broadcast_service_gateway::EitSectionAck` per-event `running_status` (3-bit); `ci_ext::resource_manager_v2` `module_id` (6-bit).

Other defects:
- **Allocation from untrusted counts.** `cicam_player::PlayerCapabilitiesReply::parse` sized its `Vec` from the 16-bit entry count before checking the body (a 6-byte APDU allocated 128 KiB). It is now bounded by the bytes present, pinned by a counting-allocator test (audit r10-O-6).
- **Panics on wire input removed** from `objects::date_time` and `ci_plus::uri` (`unwrap` / `unreachable!`) (#1091).
- **`ca_pmt` command id fabricated.** `objects::ca_pmt::write_info_block` (shared with CI Plus `MsCaPmt`) no longer invents `CaPmtCmdId::OkDescrambling` when a non-empty `CA_descriptor` loop is paired with `cmd_id: None`; it returns `Error::InvalidObject`.
- **`tpdu::ResponseTpdu::parse`** now validates the status trailer's own `t_c_id` against the TPDU body's instead of discarding it.
- **`objects::low_speed_comms::CommsCmd::serialize_into`** rejects a `command_id` / `params` combination Table 52 does not allow (for example `DisconnectOnChannel` with `CommsCmdParams::SetParams`) instead of emitting a self-inconsistent APDU.
- **DSM-CC header parsing** in `ci_ext::software_download` (the private `parse_dsmcc_header` / `parse_dsmcc_data_header`, used by every message's `parse`) now validates `protocolDiscriminator`, `dsmccType`, `messageId` (against the message type being parsed) and `messageLength`. Each message is directly `Parse`-able with no outer framing, so previously `messageId` alone let one message type's bytes silently parse as another.
- **`ci_plus::low_speed_comms_v4::CommsIpConfigReply::serialize_into`** rejects an `ip_config` / `connection_state` disagreement (`ip_config` present outside `Connected`, or absent under `Connected`); `parse` reads `ip_config` only when `connection_state == Connected`, so the other combinations serialized bytes `parse` could not reproduce.
- **Duplicated code removed.** `ci_ext::application_info_v2` and `resource_manager_v2` delegate to the EN 50221 `objects::application_info` / `resource_manager` implementations they were byte-for-byte copies of (audit r10-O-5); the local `empty_object!` / `opaque_object!` / `opaque_dsmcc_object!` macros are one shared pair (audit r10-O-4).

## Dependencies

```toml
# before (0.8.1)
broadcast-common = { ..., version = "9.3", default-features = false }
dvb-si           = { ..., version = "10",  default-features = false }
# after (0.9.0)
broadcast-common = { ..., version = "9.4", default-features = false }
dvb-si           = { ..., version = "11.0", default-features = false }
```

The `dvb-si` 10 -> 11 move changes the caret epoch of the PMT/descriptor types that the `ca_pmt` builder consumes (`dvb-si-11.0.0.md`); a program that passes `dvb_si` values into `dvb_ci::builder` must be on `dvb-si` 11 as well, or it will see two incompatible `dvb-si` crates. `broadcast-common-9.4.0.md`.

---

Published from tag `dvb-ci-v0.9.0`.
