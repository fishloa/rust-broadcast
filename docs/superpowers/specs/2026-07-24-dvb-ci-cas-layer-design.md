# dvb-ci-runtime — single-slot CI-CAM CAS orchestration layer

> Design spec for issue #763. Status: approved in brainstorming 2026-07-24; awaiting spec review before implementation-planning. Additive / non-breaking (crate stays independently versioned; this is a **minor** bump).

## Problem

`dvb-ci-runtime` today is an EN 50221 **session/resource** runtime: it manages sessions + resources (app-info, CA, MMI, date-time), takes a **raw `ca_pmt` byte buffer** (`Driver::send_ca_pmt(&[u8])`), and surfaces `CaInfo` / `CaPmtReply{descrambling_ok:bool}` / `Mmi` notifications. Everything above the protocol is left to every consumer to re-implement:

- build the `ca_pmt` from a PMT (walk CA descriptors, pick ECM/ES PIDs, encode the APDU);
- parse the CAT, select EMM PIDs per CAID, arrange for those PIDs to reach `ci0`;
- re-send the `ca_pmt` on a timer to notice entitlement changes (a card entitled *after* the initial `ca_pmt` — once activation EMMs land — never produces a new `ca_pmt_reply`, so status is stale indefinitely).

This raises the crate one level for a **single CI slot** so it owns the CA orchestration and emits high-level, typed events.

**Key framing (not host crypto):** on the CI-CAM path, ECM/EMM are processed *inside the CAM hardware*. The host's only jobs are (a) tell the CAM which ES PIDs to descramble (`ca_pmt`) and (b) feed the right TS PIDs into `ci0` (ES PIDs + the CAT's EMM PIDs). Software CAS / host ECM processing is explicitly out of scope.

## Existing building blocks (reused, not reinvented)

- `dvb-ci::builder::build_ca_pmt(pmt: &PmtSection, list_management, cmd_id) -> CaPmtBuilt` — the PMT→`ca_pmt` projection already exists.
- Multi-programme descramble API already in the runtime (`descramble_programs` / `add_program` / `remove_program`, `list_management` first/more/last/only + add/update) — a slot descrambles a **set** of services.
- `Resource::tick(elapsed)` + `next_timer`/`SetTimer` — the date-time resource already re-queries on a `response_interval`; the `ca_pmt` re-query reuses this exactly.
- `dvb_ci::objects::ca_pmt_reply::CaEnable` — typed status enum (`name()` + `impl_spec_display!` already); the `CaPmtReply` *object* already carries `Option<CaEnable>`. The runtime just plumbs it up.
- `Notification` + `pump`/`pump_hotplug`/`take_notifications` — the existing event framework (#726). New events are new `Notification` variants; **no new event system**.
- `CiDataDevice` trait — the `ci0` TS data plane (write scrambled → read descrambled), for the turnkey layer.
- `dvb_si::tables::pmt::PmtSection`, `dvb_si::tables::cat::CatSection`, `dvb_si::descriptors::ca::CaDescriptor` (tag 0x09) — parsed inputs.

## Architecture — two layers, additive

The current raw `send_ca_pmt(&[u8])` + `CaPmtReply` surface **stays**; the managed API sits alongside (opt-in).

### Layer 1 — sans-IO core (`ManagedCa`, single slot)
Added to `Driver`, no IO of its own. Fed **parsed dvb-si structs** (never raw bytes):

```rust
impl<D: CaDevice> Driver<D> {
    /// Build + send the ca_pmt for `pmt` (via dvb-ci::builder), tracking it in
    /// the slot's active service set (multi-programme list-management under the
    /// hood). Additive to raw send_ca_pmt.
    pub fn add_service(&mut self, pmt: &dvb_si::tables::pmt::PmtSection<'_>) -> Result<(), CaError>;
    /// Drop a service from the active set (list_management update / not_selected).
    pub fn remove_service(&mut self, program_number: u16) -> Result<(), CaError>;
    /// Set the current CAT; recomputes the EMM-PID set = CAT EMM PIDs ∩ the
    /// CAM's advertised ca_info CAIDs (feed only what this CAM can use).
    pub fn set_cat(&mut self, cat: &dvb_si::tables::cat::CatSection<'_>) -> Result<(), CaError>;
    /// PIDs the caller must route into ci0 (typed, owned by the driver state).
    pub fn emm_pids(&self) -> &[u16];
    pub fn descramble_pids(&self) -> &[u16];
    /// Re-query cadence (default REQUERY_DEFAULT = 10 s). 0 disables re-query.
    pub fn set_requery_interval(&mut self, interval: core::time::Duration);
}
```
`PmtSection`/`CatSection` are borrowed (`<'a>`); `add_service`/`set_cat` copy the needed fields (program_number, ES PIDs, CA descriptors + ca_pids, EMM CAID→PID) into **owned** driver state at call time, so nothing borrows across calls. PMT is single-section per ISO/IEC 13818-1 (§2.4.4.8), so `PmtSection` IS the complete PMT — no reassembly. CAT is single-section in practice (documented assumption; dvb-si's `collect` layer is available if a real multi-section CAT ever appears).

### Layer 2 — turnkey wrapper (owns `ci0`)
A thin type owning a `CiDataDevice`, driving Layer 1:
```rust
pub struct CaDescrambler<D: CaDevice, C: CiDataDevice> { /* Driver<D> + C */ }
impl<D,C> CaDescrambler<D,C> {
    pub fn add_service(&mut self, pmt: &PmtSection<'_>) -> Result<(), CaError>;
    pub fn set_cat(&mut self, cat: &CatSection<'_>) -> Result<(), CaError>;
    /// Route ES+EMM PIDs from a scrambled TS chunk into ci0, return descrambled TS.
    pub fn feed_ts(&mut self, scrambled: &[u8]) -> io::Result<Vec<u8>>;
    pub fn take_notifications(&mut self) -> Vec<Notification>;  // delegates
}
```
The caller never does PID math or byte-level `ca_pmt`/CAT work.

## Data flow
parsed `PmtSection`/`CatSection` → `ManagedCa` → `build_ca_pmt` → `CaDevice` (ci control plane). CAM `ca_pmt_reply` → `Notification::CaPmtReply{ program_number, ca_enable: CaEnable, descrambling_ok }`. `Resource::tick` at the re-query interval re-sends the active `ca_pmt`s; on a per-program `(ca_enable, descrambling_ok)` transition the core emits the edge event. Turnkey layer additionally shuttles scrambled/descrambled TS through `ci0`, routing `emm_pids() ∪ descramble_pids()`.

## Events (existing `Notification` framework)
- **New:** `Notification::Entitlement { program_number: u16, ca_enable: CaEnable, descrambling_ok: bool }` — **edge-triggered** per program (fires only on a status transition detected by the re-query), mirroring #726's edge pattern. `Notification` is `#[non_exhaustive]` → additive.
- **Enriched:** `Notification::CaPmtReply` gains `ca_enable: Option<CaEnable>` — `None` = programme `CA_enable_flag` clear (no programme-level status; defer to ES entries), `Some(_)` = the typed status (distinguishes not-entitled vs unavailable vs technical vs …). Preserves the object's own `None`-vs-`Some(Rfu(_))` distinction (do NOT collapse a clear flag to a sentinel `Rfu(0)`). `descrambling_ok: bool` stays (derived: `Some(possible*) => true`, else `false`). `program_number` already present.
- **Unchanged:** #726 `HotPlug{Cam*/Card*}` stays exactly as-is (coarse module/card layer). The per-program `Entitlement` event is the fine-grained per-service layer. Two distinct, complementary layers — documented as such.
- Delivered via the existing `pump` / `pump_hotplug` (add a `pump`-level filter is unnecessary; consumers match `Notification::Entitlement`).

## Error handling
Reuse the crate's `thiserror` set; new arms only where real:
- `add_service` with a PMT carrying no CA descriptor at program or ES level → typed `CaError::NoCaDescriptor { program_number }` (not a panic, not a silent no-op).
- `set_cat` before the CAM's `ca_info` is known → not an error; the EMM set stays empty until CAIDs arrive, then recomputes.
- No new panic paths; all parse/selection paths are `Result`.

## Testing (`MockCaDevice` + `MockCiDataDevice`, all biting)
1. `add_service(&PmtSection)` from a **real scrambled-service PMT fixture** (dvb-si corpus, with program + ES CA descriptors) → assert the sent `ca_pmt` bytes byte-match a `build_ca_pmt` oracle.
2. `set_cat(&CatSection)` → `emm_pids()` equals CAT EMM PIDs ∩ scripted `ca_info` CAIDs (and excludes CAIDs the CAM doesn't advertise).
3. Re-query edge: scripted CAM returns `descrambling_ok=false` on reply 1 and `true` on reply 2 (after the tick interval) → assert **exactly one** `Notification::Entitlement` with the right `program_number`/`ca_enable`; no event when status is unchanged across re-queries (negative control).
4. `CaPmtReply` now carries the typed `ca_enable`.
5. Turnkey `CaDescrambler` over `MockCiDataDevice` end-to-end: feed scrambled TS + PMT/CAT → the right PIDs routed into ci0, descrambled TS returned, entitlement events surfaced.
Each must fail if its logic is neutered (mutation-checked).

## Spec grounding
EN 50221 §8.4.3.4 (`ca_pmt`, Table 25) + §8.4.3.5 (`ca_pmt_reply`, Table 26) — already in `dvb-ci/docs`, EN 300 468 §6.2.16 (CA descriptor tag 0x09) + CAT (ISO/IEC 13818-1 §2.4.4.5), all already transcribed for the crates that implement them. The re-query-to-refresh-entitlement behaviour is CI operational practice (per #763); no new spec PDF required.

## Non-goals (stay in the consumer)
- **Multi-slot policy** — which CAM/slot descrambles which service, failover, per-CAM capacity. Application policy; this crate stays single-slot, the app orchestrates across slots.
- **Software CAS / host ECM (dvbcsa/newcamd)** — a separate path, not this crate.

## Migration
Fully additive: `add_service`/`set_cat`/`emm_pids`/`descramble_pids` + `CaDescrambler` sit alongside raw `send_ca_pmt` + the notification surface. `CaPmtReply` gains a field (its consumers already match a struct variant; `#[non_exhaustive]` on the enum keeps additions non-breaking). Existing consumers unaffected until they opt in. dvb-ci-runtime 0.13.0 → **0.14.0** (minor).
