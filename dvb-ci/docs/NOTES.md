# dvb-ci / dvb-ci-runtime implementation notes

The spec transcriptions (EN 50221, TS 101 699, TS 103 205) live in the private repo at `private/specs-verified/dvb/dvb-ci/` (and `.../dvb-ci-runtime/`). This note is the public, own-words summary of what the code does; the Rust source is the authority. Clauses are cited by number only.

## 1. Layers and crates

| Layer | Where | Role |
|---|---|---|
| Transport (TPDU) | `dvb-ci/src/tpdu.rs`, runtime `transport.rs` | framing of the single transport connection per slot (EN 50221 Annex A) |
| Session (SPDU) | `dvb-ci/src/spdu.rs`, runtime `session.rs` | multiplexes one session per resource (EN 50221 §7.2) |
| Application (APDU) | `dvb-ci/src/objects/`, `ci_ext/`, `ci_plus/` | typed resource objects, dispatched by 3-byte tag (`AnyApdu`) |
| Resource state machines | runtime `resource.rs`, `stack.rs` | host-side behaviour of each resource |
| Driver | runtime `driver.rs`, `managed.rs`, `descrambler.rs` | I/O pump, notifications, CAS orchestration |

`dvb-ci` is `no_std` + `alloc` and sans-IO: bytes to typed objects and back, plus the `CA_PMT` builder. `dvb-ci-runtime` is `std`, sync, and written against the `CaDevice` trait so everything runs on the in-memory `MockCaDevice` as well as the Linux device (`linux` feature, `LinuxCaDevice` / `LinuxCiDataDevice`).

Every wire type implements `Parse` / `Serialize`; a parse that would not re-serialize byte-identically is an error. The `length_field` coding form (`LengthForm`) is preserved, so a CAM that uses a long form for a short length round-trips unchanged. Reserved bits a peer sent are kept (`reserved.rs`). A tag nobody typed parses to `AnyApdu::Unknown` and round-trips losslessly.

## 2. Resource identifiers

`ResourceId(u32)` carries the 4-octet identifier verbatim. For public ids (`resource_id_type` 0..=2) it exposes class (14 bits), type (10 bits), version (6 bits); type 3 is private (`is_private()`). `ResourceId::name()` only knows the six EN 50221 base resources; anything else displays as `resource_id(0x........)`.

Constants defined in code:

| Group | Constant | Value |
|---|---|---|
| EN 50221 (`resource.rs`) | `RESOURCE_MANAGER` | `0x00010041` |
| | `APPLICATION_INFORMATION` | `0x00020041` |
| | `CONDITIONAL_ACCESS_SUPPORT` | `0x00030041` |
| | `HOST_CONTROL` | `0x00200041` |
| | `DATE_TIME` | `0x00240041` |
| | `MMI` | `0x00400041` |
| TS 101 699 (`ci_ext`) | `RESOURCE_MANAGER_V2` / `APPLICATION_INFO_V2` | `0x00010042` / `0x00020042` |
| | `POWER_MANAGER`, `APPLICATION_MMI`, `DOWNLOAD` | `0x00220041`, `0x00410041`, `0x00051041` |
| | templates: `STREAM_INPUT`, `BROADCAST_SERVICE_GATEWAY`, `STATUS_QUERY`, `EVENT_MANAGER`, `COPY_PROTECTION`, `CA_PIPELINE` | `0x00801001`, `0x00811001`, `0x00211001`, `0x00231001`, `0x00041001`, `0x00061001` |
| TS 103 205 (`ci_plus`) | `MULTISTREAM`, `CONTENT_CONTROL`, `MULTISTREAM_HOST_CONTROL`, `HOST_CONTROL_V3`, `SAMPLE_DECRYPTION`, `CICAM_PLAYER`, `FILE_RETRIEVAL` | `0x00900041`, `0x008C1041`, `0x00200081`, `0x00200043`, `0x00920041`, `0x00930041`, `0x00910041` |

The "template" ids carry a module id inside the identifier; `ci_ext::MODULE_ID_MASK` (`0xFFFFF03F`) and `module_id()` mask that field out so a handler can match the resource regardless of which module instance it is.

The extension resources reuse `0x9F80xx` tags, so the tag alone is ambiguous: `CiExtApdu` and `CiPlusApdu` dispatch on the resource id of the session, not on the tag. Extended CA Support, LSC v4, URI v3 and the Sample-Mode descriptors are standalone typed structs because TS 103 205 gives them no resource id (per the crate docs).

Typed coverage in `dvb-ci`:
- `objects/`: Resource Manager, Application Information, CA Support (`ca_info`, `ca_pmt`, `ca_pmt_reply`), Host Control, Date-Time, MMI low-level (`mmi_close`, `mmi_display`) and high-level (`mmi_high`), Low-Speed Communications. `AnyApdu` types 40 tags.
- `ci_ext/`: the TS 101 699 set listed above.
- `ci_plus/`: framing and typed bodies only; see section 6.

## 3. Runtime state flow

### Transport (`transport.rs`, EN 50221 Annex A.4)

States: `Idle -> Creating -> Active` (`TcState`). `init()` sends `Create_T_C` on `t_c_id = 1`; a `C_T_C_Reply` moves it to `Active`; no reply within the reply timeout surfaces `TransportError::SetupTimeout`. Defaults are `DEFAULT_POLL_INTERVAL` = 100 ms and `DEFAULT_REPLY_TIMEOUT` = 1000 ms; the spec requires regular polling but fixes no interval, so these are implementation choices.

While `Active`, the host is the only initiator (half duplex): a poll is an empty `T_Data_Last`. Each response carries a status byte (`T_SB`, parsed with a real `length_field` decode, not an assumed `0x02`); when it reports data available the host sends `T_RCV`. Chained module data (`T_Data_More`... then `T_Data_Last`) is reassembled into one SPDU payload. Time is injected via `Event::Tick`; the core never reads a clock.

### Session (`session.rs`)

`SessionLayer` is a `session_nb -> ResourceId` table with an allocator starting at 1 (0 is reserved). It answers `open_session_request`, supports host-initiated `create_session`, handles `close_session`, and routes `session_number` + APDU up to the bound resource. Which resources the host offers is policy passed in as a predicate.

### Resources (`stack.rs`, `resource.rs`)

`CiStack::new()` advertises and implements six resources, all as host-provided: Resource Manager, Application Information, Conditional Access Support, Date-Time, MMI, Host Control. The module opens the sessions; the host accepts. The code comments record that a live AlphaCrypt module ignored host-initiated opens for these and rejected `create_session`, which is why all six are advertised.

Handshake as implemented:

1. `HostRequest::Init` resets the stack (new session table, cleared cached CAIDs, `reset()` on every handler), emits `Reset`, `QuerySlot`, then starts the transport.
2. Module opens the Resource Manager session; the host sends `profile_enq`, replies to the module's `profile_enq` with its six ids, and records the module's `profile`.
3. Once the module's profile is in, the host sends `profile_change` and emits `Notification::CamReady`. A later `profile_change` from the module re-enquires and clears readiness.
4. Application Information opens: host sends `application_info_enq`; reply becomes `Notification::ApplicationInfo`.
5. CA Support opens: host sends `ca_info_enq`; reply becomes `Notification::CaInfo` and the stack caches the CAIDs as the filter set.
6. Date-Time answers `date_time_enq` with current UTC (MJD plus BCD) and re-sends on the enquiry's interval via tick.
7. MMI menus and enquiries surface as `Notification::Mmi`; answers go back through `Driver::mmi_menu_answer` / `mmi_enquiry_answer` / `mmi_cancel`.
8. Host Control `tune` / `replace` / `clear_replace` / `ask_release` are decoded and surfaced as `Notification::HostControl`. The runtime never retunes and sends no reply APDU.

A CAM-originated APDU that fails to parse is reported as `Notification::Error`, not dropped.

### Hot-plug (`event.rs`, `driver.rs`)

`HotPlug::CamPresent` / `CamRemoved` are real signals: the driver watches the slot's module-present flag (via `SlotInfo`) and acts on the edge, re-running `Init` on insert and discarding session state on removal. `CardInserted` / `CardRemoved` / `CardChanged` are inferences, because the CI link has no card-detect line. Inputs are: the `ca_info` CAID set going empty/non-empty/different, `ca_pmt_reply` descrambling flipping, and (removal/presence) keyword matches on MMI text (`MMI_CARD_ABSENT_KEYWORDS`, `MMI_CARD_PRESENT_KEYWORDS`). Treat them as best effort. `pump_hotplug` delivers only these transitions to a closure.

## 4. CA_PMT rules

### Builder (`dvb-ci/src/builder.rs`, EN 50221 §8.4.3.4)

- Input is a `dvb_si` `PmtSection`. Every non-`CA_descriptor` (tag `0x09`) is dropped from both the programme loop and each ES loop; surviving descriptors are re-emitted as verbatim tag/length/body.
- Every elementary stream of the PMT is carried (stream type + PID) even if it has no surviving CA descriptor.
- `ca_pmt_cmd_id` is attached to a loop only when that loop has surviving descriptors; an empty loop has length 0 and no cmd byte.
- `build_ca_pmt` keeps all CA descriptors. `build_ca_pmt_for_caids` keeps only those whose first two body bytes (`CA_system_id`) are in `allowed`; a descriptor too short to read a CAID is dropped; an empty list drops all. Rationale in the code: a CAM rejects a whole `ca_pmt` containing a CAID it does not support. `CaPmtBuilt::retain_caids` filters after the fact.
- `CaPmtBuilt::to_bytes` panics on an unencodable loop (for example `program_info_length` over its 12-bit field); `try_to_bytes` returns the error and is what runtime paths use.
- Enums: `CaPmtListManagement` = More 0, First 1, Last 2, Only 3, Add 4, Update 5 (else `Reserved`); `CaPmtCmdId` = OkDescrambling 1, OkMmi 2, Query 3, NotSelected 4 (else `Rfu`).

### Runtime use (`stack.rs`, `driver.rs`)

| Call | list_management | cmd_id |
|---|---|---|
| `descramble(pmt)` | Only | OkDescrambling |
| `descramble_programs(pmts)` | Only if one, else First / More... / Last | OkDescrambling |
| `add_program(pmt)` | Add | OkDescrambling |
| `remove_program(pmt)` | Update | NotSelected |
| `add_service(pmt)` | Only if managed set empty, else Add | OkDescrambling |
| `update_service(pmt)` | Update | OkDescrambling |
| `remove_service(n)` | Update (via `remove_program`) | NotSelected |
| re-query tick | Update, sent as Query then OkDescrambling | both |

CAID filtering uses the CAM's `ca_info` CAIDs once known; before `ca_info` arrives, nothing is filtered.

### Managed CAS layer (`managed.rs`, `Driver`)

`add_service` rejects a PMT with no CA descriptor at either level (`CaError::NoCaDescriptor`) and records the service with its raw PMT bytes. `remove_service` on an unknown programme is a no-op. `set_cat` takes a parsed CAT; EMM PIDs are the CAT's `CA_descriptor` PIDs intersected with the CAM's CAIDs, recomputed automatically if `ca_info` arrives after the CAT. Derived sets: `descramble_pids` (ES PIDs), `ca_pids` (ECM PIDs from programme and ES CA descriptors), `emm_pids`, and `required_pids` = those plus each service's PCR PID plus the CAT PID.

Periodic re-query (`set_requery_interval`, default `REQUERY_DEFAULT` = zero = off) rebuilds each active service from its stored PMT every tick and sends the Query then OkDescrambling pair, so the programme is never left barred. A programme-level `CA_enable` change between replies produces the edge-triggered `Notification::Entitlement`. Per-service failures are held and returned after healthy services were sent.

`CaDescrambler` wraps a `Driver` plus a `CiDataDevice` (the `ci0` TS path). `feed_ts` keeps only packets whose PID is in `required_pids()` and returns the descrambled TS; PAT and PMT are not fed because the CAM gets the PMT through `ca_pmt`. One `CaDescrambler` is one slot and one input TS; merging multiplexes is out of scope.

## 5. Judgment calls

- Poll and reply timing are chosen, not specified.
- Session opening direction was set by hardware behaviour (section 3), not by a reading of the spec alone.
- `Init` rebuilds all stack state, so a re-inserted CAM never meets session numbers from its predecessor.
- The periodic re-query is opt-in; the code notes a live Irdeto module never answered `Query`, which is why the follow-up OkDescrambling is unconditional.
- Host Control is surfaced, not acted on.

## 6. Known gaps

- CI Plus: `dvb-ci` types the TS 103 205 APDUs (Content Control APDUs and SAC datatypes, Multi-stream, Sample Decryption, CICAM Player, file retrieval, LSC v4, URI v3) but implements no cryptography (authentication, key derivation, SAC encryption). Per its docs, `FileRequest` / `FileAcknowledge` bodies and `tune_broadcast_req` / `tune_reply` / `ask_release(_reply)` are deferred to a CI Plus edition that is not transcribed and are not encoded; DRM payloads are opaque. CI+ over USB (TS 103 605) and the PC-Card physical layer are out of scope. The runtime does not run any CI Plus resource.
- The runtime implements no Low-Speed Communications and no TS 101 699 resource (they are codec-only).
- Hardware verification is limited to what the runtime crate docs record: one AlphaCrypt/Irdeto module (handshake, `application_information`, `ca_info`) and a CXD2099 `ci0` data plane. Periodic re-query is not verified on any CAM.
- The Linux device is gated on `target_os = "linux"` and is not exercised on macOS.
