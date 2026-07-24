# dvb-ci-runtime 0.14.0

**Single-slot managed CAS layer (#763).** A CI-CAM conditional-access
orchestration layer on top of the raw `ca_pmt`/`ca_pmt_reply` runtime, fed
parsed `dvb-si` structures instead of raw APDU bytes. Fully additive — the
existing `send_ca_pmt(&[u8])` + notification surface is unchanged; the managed
API is opt-in.

## Why

Until now the crate handed you the EN 50221 session/resource machinery and a
raw `ca_pmt` byte buffer — every consumer re-implemented the layer above it:
build the `ca_pmt` from a PMT, select EMM PIDs from the CAT per CAID, route the
right TS PIDs into `ci0`, and re-send the `ca_pmt` on a timer to notice
entitlement changes (a card entitled *after* the first `ca_pmt` otherwise never
produces a fresh `ca_pmt_reply`, so its status stays stale). This release
raises the crate one level, for a single CI slot, so it owns that orchestration
and emits typed, high-level events.

On the CI-CAM path, ECM/EMM are processed *inside the CAM hardware*. The host's
only jobs are (a) tell the CAM which ES PIDs to descramble (`ca_pmt`) and (b)
feed the right TS PIDs into `ci0`. Software CAS / host ECM processing is
explicitly out of scope.

## What's new

- **Layer 1 — sans-IO core on `Driver`** (parsed structs in, no raw bytes):
  - `add_service(&PmtSection)` / `remove_service(program_number)` — build + send
    the `ca_pmt` (via `dvb_ci::builder::build_ca_pmt`) and track the slot's
    active service set; list-management auto-selected (`Only`/`Add`,
    `Update`/`NotSelected`). CA-free PMT → `CaError::NoCaDescriptor`.
  - `set_cat(&CatSection)` — EMM-PID feed = CAT EMM PIDs ∩ the CAM's `ca_info`
    CAIDs (feed only what this CAM can use).
  - `emm_pids()` / `descramble_pids()` / `ca_pids()` / `required_pids()` — the
    PIDs to route into `ci0` (EMM ∪ ES ∪ ECM = `required_pids`).
  - `set_requery_interval(Duration)` — periodic `ca_pmt` re-query with
    `cmd_id = query` (only `query`/`ok_mmi` solicit a reply per EN 50221
    §8.4.3.5) so post-`ca_pmt` entitlement changes still surface; `ZERO`
    disables. Default 10 s.
  - `Notification::Entitlement { program_number, ca_enable, descrambling_ok }` —
    edge-triggered per programme on a status transition. Complements the coarse
    #726 `HotPlug` module/card layer.
  - `Notification::CaPmtReply` gains a typed `ca_enable: Option<CaEnable>`
    (not-entitled / technical / purchase-dialogue vs the boolean-only
    `descrambling_ok`, which stays as a derived convenience). `None` = the
    programme `CA_enable_flag` was clear — never a sentinel.

- **Layer 2 — turnkey `CaDescrambler<D, C>`** owning the `ci0` `CiDataDevice`:
  `feed_ts(scrambled)` filters a scrambled TS chunk to `required_pids()`,
  writes only those packets to `ci0`, and returns the descrambled TS.
  `add_service`/`set_cat`/`take_notifications`/`required_pids` delegate to the
  wrapped `Driver`. **One `CaDescrambler` = one CI slot = one TS path** — a
  multi-tuner setup uses one per slot; merging services across muxes into a
  single slot is a remux job (PID collisions), out of scope here.

## Compatibility

Additive / non-breaking. `Notification` is `#[non_exhaustive]`, so the new
variant and the `CaPmtReply` field don't break existing matches. Existing
consumers of the raw `send_ca_pmt` + notification API are unaffected until they
opt into the managed layer. `dvb-ci-runtime` is independently versioned:
0.13.0 → 0.14.0 (minor).

## Spec grounding

EN 50221 §8.4.3.4 (`ca_pmt`, Table 25) + §8.4.3.5 (`ca_pmt_reply`, Table 26);
CA descriptor (ETSI EN 300 468 §6.2.16, tag `0x09`) + CAT (ISO/IEC 13818-1
§2.4.4.5). The re-query-to-refresh-entitlement behaviour is CI operational
practice.
