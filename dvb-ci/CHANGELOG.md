# Changelog

All notable changes to `dvb-ci` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/); this crate adheres to semantic
versioning.

## Unreleased

### Added

DVB CI Extensions (ETSI TS 101 699) foundation — a new `ci_ext` module tree with
**resource-scoped** APDU dispatch ([`ci_ext::CiExtApdu`]). Unlike EN 50221, the
TS 101 699 resources reuse the same `0x9F80xx` tag values across resources
(Table 87), so they cannot join the global `AnyApdu`: `CiExtApdu::parse` keys on
the `resource_identifier()` first (masking out the 6-bit Module ID for `type = 1*`
resources via `classify`/`MODULE_ID_MASK`), then dispatches on the leading
`apdu_tag` within the selected resource. Each object is symmetric
`Parse`/`Serialize` with biting round-trip + field-mutation tests; serialize
rebuilds every byte from typed fields (no raw passthrough). Resources in this
pass:

- **Resource Manager v2** (§4.2.1, Tables 3-7, `0x00010042`): `ProfileEnq`,
  `ProfileReply`, `ProfileChanged`, `ModuleIdSend`, `ModuleIdCommand` (with
  `ModuleIdCommandKind`).
- **Application Information v2** (§5, Table 11, `0x00020042`): `ApplicationInfoEnq`,
  `ApplicationInfo`, `EnterMenu` — with the extended `ApplicationTypeV2` value set
  (Software_upgrade / Network_interface / Accessibility_aids / Unclassified) and
  the §5.1.2 "unrecognized → Unclassified" rule (`effective()`).
- **Power Manager** (§6.3, Tables 52-55, `0x00220041`): `ActivationStateChangeRequest`,
  `ActivationStateChangeAck` — with `ActivationState` and `ReplyCode`.
- **Event Manager** (§6.4, Tables 56-61, `0x00231ii1`): `EventRequest`,
  `EventRequestAck`, `EventNotification` — with `EventType` and `EventReply`.
- **Copy Protection** (§6.6, Tables 69-73, `0x00041ii1`): `CpQuery`, `CpReply`,
  `CpCommand`, `CpResponse` — with `CpStatus`.
- **StreamInput** (§6.1.2, Tables 12-20, `0x00801ii1`) — Type 'A' (TS-level) input
  modules: `DeliverySystemInfoReq`/`DeliverySystemInfoAck` (a `SystemIdentifier`
  list — Abstract / DVB-C / DVB-S / DVB-T), `ScanStartReq`, `ScanNextReq`
  (`9F8003`), `ScanAck` (`TSState` + 11-byte `TuningInformationMessage` +
  `ScanProgress`), `TuneTSReq` (optional 11-byte tuning message; absent =
  disconnect), `TuneTSAck`.
- **ServiceGateway** (Generic Service Gateway, §6.1.3, Tables 21-31) — Type 'B'
  service-level access (never instantiated alone; inherited by network-specific
  gateways): `ServiceListReq`/`ServiceListAck` (a `{OriginalNetworkID, ServiceID}`
  service-reference list), `ServiceListVersionReq`/`ServiceListVersionAck`,
  `ServiceListChanged`, `ServiceDescReq`/`ServiceDescAck` (SDT-modelled params —
  `running_status` resolved to **3 bits** per the byte budget — + verbatim SDT
  descriptor loop), `GetServiceReq`/`GetServiceAck` (with `EitResponseCode`-style
  flag bits + `ActualService`).
- **BroadcastServiceGateway** (§6.1.3.3, Tables 32-34, `0x00811ii1`) — Type 'B' on
  a broadcast network: `EITSectionReq`, `EITSectionAck` (`EitResponseCode` +
  EIT-modelled event loop with verbatim event descriptor loops). It **inherits all
  Generic Service Gateway calls**: `BroadcastServiceGatewayApdu` routes
  `9F8010`/`9F8011` to the EIT objects and delegates every other `9F80xx` tag to
  the wrapped `ServiceGatewayApdu`.
- **Status Query** (§6.2, Tables 35-51, `0x00211ii1`): `StatusQueryReq`, `TrapReq`,
  `GetNextItemReq`, `GetNextItemAck` (32-bit `StatusItem`s), `StatusAck` (a
  `StatusItem` + opaque `StatusBytes`) — with the `StatusItem` value enum. Plus the
  audience-metering item-content structures decoding those `StatusBytes`:
  `SelectionInformation` (Table 43, the nested input-port → output-port routing
  loop), `PortProfile` (Table 48), `ViewedService` (Table 49) and
  `ActivationStatus` (Table 50, with the `ActivationState` enum).
- **Application MMI** (§6.5, Tables 62-68, `0x00410041`): `RequestStart` (two
  length-prefixed strings), `RequestStartAck` (with the `AckCode` enum), `FileReq`,
  `FileAck` (`FileOK` flag + file bytes), `AppAbortReq`, `AppAbortAck`.
- **Download** (CAM firmware, §6.7, Tables 74-83, `0x00051041`): `DownloadEnquiry`,
  `DownloadReply` (each encapsulating one opaque DSM-CC U-N message),
  `UserAuthInitiate`, `UserAuthResult` (the 7-byte `BinaryId` + opaque payload).
  Plus the DSM-CC U-N Download message structures (ISO/IEC 13818-6, Tables 79-83):
  `DownloadInfoRequest`, `DownloadInfoResponse` (multi-module loop),
  `DownloadCancel`, `DownloadDataRequest`, `DownloadDataBlock` — with the firmware
  payload, compatibility-descriptor bodies, module info and private data carried as
  opaque borrowed `&[u8]` per §6.7.5.
- **CA Pipeline** (§6.8, Tables 84-86, `0x00061ii1`): `CaPipelineRequest`,
  `CaPipelineResponse`, `CaPipelineNotification` — each carrying an opaque
  `CASpecificData` byte blob (CA-system-specific encoding).

The remaining EN 50221 application objects, completing every Table 58 `apdu_tag`.
All are symmetric `Parse`/`Serialize` with biting round-trip + mutation tests and
PDF worked-example fixtures; serialize rebuilds every byte from typed fields (no
raw passthrough). The `_last`/`_more` chaining pairs are modeled as a single
struct with a `more: bool` that selects the tag and dispatches both tags to the
same `AnyApdu` variant.

- **Host Control** (§8.5.1, Tables 27-30): `Tune`, `Replace`, `ClearReplace`,
  `AskRelease`.
- **High-level MMI** (§8.6.5, Tables 46-51): `Text` (last/more), `Enq`, `Answ`
  (with `AnswId`), `Menu` (last/more), `MenuAnsw`, `List` (last/more); the nested
  `TEXT()` component is reused across Menu/List.
- **Low-level / display / scene / download MMI** (§8.6.2-8.6.4, Tables 34-45):
  `DisplayControl`, `DisplayReply` (graphics-characteristics / character-table
  list / mmi_mode_ack branches), `KeypadControl`, `Keypress`, `SubtitleSegment`
  (last/more), `DisplayMessage`, `SceneEndMark`, `SceneDoneMessage`,
  `SceneControl`, `SubtitleDownload` (last/more), `FlushDownload`, `DownloadReply`
  — with the `DisplayControlCmd`, `MmiMode`, `DisplayReplyId`, `KeypadControlCmd`,
  `DisplayMessageId` and `DownloadReplyId` value enums.
- **Low-Speed Communications** (§8.7.1, Tables 52-56): `CommsCmd` (with nested
  `ConnectionDescriptor`), `ConnectionDescriptor`, `CommsReply`, `CommsSend`
  (last/more), `CommsRcv` (last/more) — with the `CommsCommandId`,
  `ConnectionDescriptorType` and `CommsReplyId` value enums.

CI Plus extensions (ETSI TS 103 205) foundation — a new `ci_plus` module tree with
its own **resource-scoped** APDU dispatch ([`ci_plus::CiPlusApdu`]). CI Plus
resources have their own apdu_tag namespace that collides with EN 50221 / TS 101 699
(e.g. Multi-stream `0x9F92xx`, Content Control `0x9F90xx`, and the extended CA
Support `ca_pmt`/`ca_pmt_reply` reuse `0x9F8032`/`0x9F8033`), so they cannot join
the global `AnyApdu`: `CiPlusApdu::parse` keys on the `resource_identifier()` first
(`classify`/`CiPlusResource`), then dispatches on the leading `apdu_tag` within the
selected resource. Every object is symmetric `Parse`/`Serialize` with biting
round-trip + field-mutation + ≥2-element boundary tests; serialize rebuilds every
byte from typed fields (no raw passthrough). This pass:

- **Multi-stream resource** (§6.4.2, Tables 2-5, `0x00900041`):
  `CicamMultistreamCapability` (`9F9200`), `PidSelectReq` (`9F9201`, multi-PID loop
  with `critical_for_descrambling_flag`), `PidSelectReply` (`9F9202`, multi-PID loop
  with `PID_selection_flag` + per-PID `PID_selected_flag`).
- **Content Control multi-stream** (§6.4.3, Tables 6-13, `0x008C1041`): the
  printed-syntax extended APDUs `CcPinReply` (`9F9014`, optional `LTS_id` via
  `LTS_bound_flag`) and `CcPinEvent` (`9F9015`); plus the SAC protocol datatype
  model (`SacMessage`/`SacDatatype` with the `DatatypeId` enum — `LTS_id`=50,
  `cicam_license`=33, `PINcode`=39, `uri_message`=25, … — and the `OperatingMode`
  enum). Crypto/license/PIN payloads carried as opaque borrowed `&[u8]`. The
  remaining Table 6 APDUs defer to CI Plus V1.3 and are not encoded.
- **CA Support multi-stream** `ca_pmt`/`ca_pmt_reply` (§6.4.4, Tables 14/16):
  `MsCaPmt` (leading `LTS_id` + added `PMT_PID`) and `MsCaPmtReply` (leading
  `LTS_id`), as **standalone** directly-constructible/parseable typed structs with a
  `CaSupportApdu::parse(tag, body)` helper. TS 103 205 does **not** print a
  resource_id for the `resource_type = 2` variant (defers to CI Plus V1.3), so these
  are intentionally not wired into `CiPlusApdu`'s resource dispatch — no resource_id
  is invented.
- **CI Plus Sample-Mode descriptors** (§7.5.5.4, Tables 46/47):
  `CiplusInitializationVectorDescriptor` (`0xD0`) and
  `CiplusKeyIdentifierDescriptor` (`0xD1`) — TLV descriptors (`descriptor_tag` +
  `descriptor_length` + opaque body).
- **Multi-stream Host Control** (§6.4.5, Tables 17-22; base DVB Host Control v3
  §13.2, Tables 97-102; `0x00200081` / base v3 `0x00200043`): the implemented tune
  APDUs `TuneTripletReq` (`9F8409`), `TuneLcnReq` (`9F8407`), `TuneIpReq`
  (`9F8408`), `TunerStatusReq` (`9F840A`, header-only) and `TunerStatusReply`
  (`9F840B`, `num_dsd` loop) — each adding the §6.4.5 `background_tune_flag` to the
  base-v3 layout. The **two resource ids** both route to the
  `CiPlusResource::MultistreamHostControl` kind, carrying a `HostControlMode`
  (`MultiStream` vs `BaseV3`) that selects the **`tune_ip_req` reserved-bit
  divergence** (Table 21 = `reserved(1)` + `background_tune_flag`; Table 100 =
  `reserved(2)`, no `background_tune_flag`) — both layouts are modeled by the one
  `TuneIpReq` carrying its `mode`, serializing to distinct byte patterns.
  `tune_broadcast_req` / `tune_reply` / `ask_release(_reply)` are **deferred to CI
  Plus V1.3** (tags not printed in TS 103 205) and intentionally not encoded.
- **Sample decryption** (§7.4, Tables 30-39, `0x00920041`): `SdInfoReq`
  (`9F9800`, header-only) / `SdInfoReply` (`9F9801`, `drm_system_id` + 128-bit
  `drm_uuid` lists) / `SdStart` (`9F9802`) / `SdStartReply` (`9F9803`, with the
  `TransmissionStatus` and `DrmStatus` value enums) / `SdUpdate` (`9F9804`) /
  `SdUpdateReply` (`9F9805`). `SdStart`/`SdUpdate` share the `ts_flag`-selected
  `SamplePayload` (TS-level metadata-record loop vs per-`track_PID` Sample-Track
  loop of `DrmMetadataRecord`s); the `drm_metadata_byte` blobs (pssh/sinf/CASD/MPD/
  OSDT per Table 34) are opaque borrowed `&[u8]`. (`CiPlusApdu` gains a lifetime
  parameter to carry the borrowed Host-Control / Sample-decryption bodies.)

## 0.1.0 — 2026-06-20

### Added

Initial release — DVB Common Interface (ETSI EN 50221) wire protocol, `#![no_std]`
(+ `alloc`). Every type is symmetric `dvb_common::Parse` / `Serialize` with
length fields computed from content and biting round-trip tests.

- **APDU framework**: `ApduTag` (3-byte ASN.1 tag) with named Table 58 constants;
  `length::{decode, encode_into, encoded_len}` (the EN 50221 ASN.1-style
  `length_field`, §7 Table 1); `AnyApdu` tag dispatch built from a single
  `declare_apdus!` list (Def-trait + drift test, mirroring `dvb-si`/`dvb-scte35`);
  4-octet `ResourceId` with the public Table 57 resource constants.
- **CA support** (§8.4.3): `ca_info_enq` / `ca_info`, `ca_pmt`, `ca_pmt_reply`,
  with the `ca_pmt_list_management`, `ca_pmt_cmd_id` and `CA_enable` value enums.
- **CA_PMT builder** (`builder::build_ca_pmt`): projects a `dvb-si` `PmtSection`
  into a `ca_pmt`, stripping all non-CA descriptors and keeping `CA_descriptor`s
  (tag `0x09`) at programme + ES level per §8.4.3.4. Tested against a real
  TSDuck-captured PMT and a multi-ES CA-protected PMT.
- **Application Information** (§8.4.2): `application_info_enq` / `application_info`
  / `enter_menu`, with the `application_type` enum.
- **Resource Manager** (§8.4.1): `profile_enq` / `profile` (reply) /
  `profile_change`.
- **Date-Time** (§8.5.2): `date_time_enq` / `date_time` (optional signed
  `local_offset`).
- **MMI** (§8.6.2.1): `close_mmi` with the `close_mmi_cmd_id` enum.
- **Session layer SPDUs** (§7.2): `open` / `create` / `close` session
  request + response, `session_number`, and the `session_status` value enum.
- **Transport layer TPDUs** (Annex A §A.4.1): C_TPDU / R_TPDU (with the mandatory
  Status Byte), the connection-management objects (Create/Delete/Request/New_T_C,
  C_T_C_Reply/D_T_C_Reply, T_C_Error) and `SB_value`.
- `examples/`: `build_ca_pmt` and `parse_apdu`.

### Deferred to a follow-up

- MMI **high-level** objects (text / enq / answ / menu / list, Tables 46-51) and
  the MMI low-level/display objects.
- **Host Control** (tune / replace) and **Low-Speed Communications** resources.

Their `apdu_tag`s are retained in `docs/en_50221/apdu-tag-values.md`; until typed
they parse as `AnyApdu::Unknown` (raw body preserved, lossless round-trip). CI+
crypto (the CC resource) and the PC-Card hardware transport remain out of scope.
