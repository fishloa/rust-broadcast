# RFC 8445 — Interactive Connectivity Establishment (ICE)

Source: `https://www.rfc-editor.org/rfc/rfc8445.txt`, July 2018. Obsoletes
RFC 5245. Section numbers below are RFC 8445's own.

Implemented by the external `rtc-ice` crate underneath
[`webrtc_runtime::media::transport::MediaTransport`]'s `IceAgent` field.
This crate's own code in `transport.rs` configures that agent (candidate
types, ufrag/pwd, controlling role) and translates its events, but the
checklist/pair-state machine, priority math, nomination, and role-conflict
handling described below all execute inside `rtc-ice`, not here. This
transcription exists to check `MediaTransport::new`'s configuration
choices against the spec, and to give a from-spec reference for anything
built directly on top of `rtc-ice` events later.

## 1. STUN attributes ICE adds (§7.1, formally defined §16.1)

These four attributes are **ICE's** extension to STUN (RFC 8489 defines
none of them — see the note at the top of `rfc8489-stun.md`):

| Attribute | Type | Width | Notes |
|---|---|---|---|
| PRIORITY | `0x0024` | 32-bit unsigned int | §7.1.1: MUST be in every Binding **request**, set via the formula in §5.1.2.1 **using the peer-reflexive type preference** (not the local candidate's own type) — this is the priority a peer-reflexive candidate discovered by this check would get. |
| USE-CANDIDATE | `0x0025` | 0 (flag, no value) | §7.1.2: controlling agent MUST include it to nominate a pair (§8.1.1); controlled agent MUST NOT ever include it in a request. |
| ICE-CONTROLLED | `0x8029` | 64-bit unsigned int (tiebreaker) | §7.1.3/§16.1: MUST be in every request sent by an agent in the controlled role. |
| ICE-CONTROLLING | `0x802A` | 64-bit unsigned int (tiebreaker) | §7.1.3/§16.1: MUST be in every request sent by an agent in the controlling role. |

Tiebreaker values (ICE-CONTROLLED/-CONTROLLING content): a random 64-bit
number, held constant for every Binding request across all streams for
the session, **except** it MUST change after a 487 role-conflict response
(§7.2.5.1) and MAY change on an ICE restart.

## 2. Candidate types, foundation, priority (§5.1.1–5.1.2)

### 2.1 Types (§5.1.1)

Four candidate types, in RECOMMENDED type-preference order (§5.1.2.2):
**host** (126) > **peer-reflexive** (110) > **server-reflexive** (100) >
**relayed** (0). Type preference MUST be identical within a type, distinct
across types, and peer-reflexive's preference MUST exceed
server-reflexive's. `0` means "last resort only."

- **Host** (§5.1.1.1): bind to a local IP/port. Base = itself. Excluded:
  loopback addresses, deprecated IPv4-compatible IPv6, IPv6 site-local.
  IPv4-mapped IPv6 SHOULD be excluded unless the app is IPv6-only.
- **Server-reflexive / relayed** (§5.1.1.2): via STUN Binding (srflx) or
  TURN Allocate (relayed + srflx together). Binding requests to a STUN
  server are **not authenticated**, and any ALTERNATE-SERVER in the
  response is ignored. Base of a server-reflexive candidate = the host
  candidate the request was sent from. Base of a relayed candidate =
  itself.
- **Peer-reflexive**: never gathered up front — learned only as a
  byproduct of connectivity checks (§7.2.5.3.1/§7.3.1.3), when a check
  arrives from/returns a mapped address that doesn't match any known
  remote candidate.

### 2.2 Foundation (§5.1.1.3)

Two candidates share a foundation iff: same type, same base IP (port may
differ), same STUN/TURN server IP (for reflexive/relayed), and same
transport protocol. Any difference in those ⇒ different foundation.

### 2.3 Priority formula (§5.1.2.1)

```
priority = (2^24) * (type preference) +
           (2^8)  * (local preference) +
           (2^0)  * (256 - component ID)
```

- Type preference: integer `0..126` inclusive (constraints above).
- Local preference: integer `0..65535` inclusive; unique per candidate of
  the same type on the same component/stream if there are several (e.g.
  multihomed); RECOMMENDED `65535` when there's only one IP.
- Component ID: integer `1..256` inclusive. RTP=1, RTCP=2 when not
  rtcp-mux'd; with rtcp-mux, component ID 1 covers both.
- Result is guaranteed a positive integer in `1..2^31-1` (a candidate's
  priority MUST be unique per data stream).

### 2.4 Pair priority (§6.1.2.3)

```
pair_priority = 2^32 * MIN(G,D) + 2*MAX(G,D) + (G>D ? 1 : 0)
```

where `G` = the controlling agent's candidate's priority, `D` = the
controlled agent's candidate's priority. Ties broken arbitrarily.
**Depends on role** — a role switch (§7.2.5.1) requires recomputing every
pair priority.

## 3. Checklists and pair states (§6.1.2)

### 3.1 Checklist states (§6.1.2.1)

- **Running** — default; neither Completed nor Failed.
- **Completed** — has a nominated pair for every component of the stream.
- **Failed** — no valid pair for some component, and every pair for that
  component has reached Failed or Succeeded (i.e. that component can
  never produce a valid pair anymore).

### 3.2 Pair states (§6.1.2.6, FSM in Figure 6)

`Frozen → (unfreeze) → Waiting → (perform check) → In-Progress → (result)
→ {Succeeded | Failed}`.

Initial-state assignment algorithm (§6.1.2.6): every pair in the whole
checklist **set** starts Frozen; then, walking checklists in the usage's
own defined order, for **each foundation** exactly one pair carrying it
(the lowest-component-ID / then-highest-priority one, in the first
checklist that has it) is moved to Waiting — so after this pass, one
pair per foundation across the whole set is Waiting, not per-checklist.

### 3.3 Pruning and pair-count cap (§6.1.2.4–6.1.2.5)

- A reflexive local candidate in a pair is replaced by its **base**
  before the check is sent (you can't originate traffic from a reflexive
  address, only receive it there).
- Pairs redundant with a higher-priority pair (same local base + same
  remote candidate) are pruned.
- **Default cap: 100 candidate pairs across the whole checklist set**,
  MUST be configurable, enforced by evenly discarding lowest-priority
  pairs per checklist until under the cap. Exists specifically to bound
  the amplification-style DoS described in §19.5.1.

### 3.4 Scheduling checks (§6.1.4)

- Ordinary + triggered checks are paced by timer **Ta** (§14.2 below); one
  check per Ta tick, round-robining across checklists in the Running
  state.
- Each tick: (1) if the triggered-check queue for the picked checklist is
  non-empty, pop and check that pair first; else (2) if there's a Frozen
  pair whose foundation has no Waiting/In-Progress pair anywhere in the
  set, unfreeze it to Waiting; else (3) pick the highest-priority Waiting
  pair (ties → lowest component ID) and check it; else (4) nothing to do
  for this checklist this tick — move on to the next Running checklist
  without waiting for Ta again.
- Message-integrity for a check uses the **peer's** ufrag/pwd (learned
  from the candidate exchange), combined per §7.2.2 below — not the
  local agent's own credential.

## 4. Role, credentials, connectivity checks (§6.1.1, §7.2)

### 4.1 Role determination (§6.1.1)

- Both full: the session **initiator** is controlling, the other
  controlled.
- One full, one lite: the full agent is always controlling.
- Both lite: initiator is controlling; if both believe themselves
  controlling, resolve via the signalling protocol's own glare detection
  — outside ICE's scope.
- Role persists for the session's life; only redetermined on ICE restart,
  and even then only if (a) the controlling agent was full and switches
  to lite while the peer is full, or (b) a 487 role-conflict forces it
  (§7.3.1.1). An agent MUST accept a peer-initiated redetermination even
  if these criteria aren't met (RFC 5245 back-compat).

### 4.2 Forming credentials for a check (§7.2.2)

```
username = <peer's ufrag> ":" <this agent's ufrag>
password = <peer's password>
```

Worked example straight from the spec: initiator L (`LFRAG`/`LPASS`),
responder R (`RFRAG`/`RPASS`). L→R check: username `RFRAG:LFRAG`,
password `RPASS`. R→L check: username `LFRAG:RFRAG`, password `LPASS`.
Responses carry no USERNAME attribute at all (§7.2.2) but reuse the same
password for MESSAGE-INTEGRITY verification.

### 4.3 Sending checks (§7.2.4)

Short-term STUN credential mechanism is mandatory for connectivity-check
Binding requests. **RFC 3489 backwards-compatibility MUST NOT be assumed**
and **FINGERPRINT MUST be used** for every connectivity check (contrast
gathering's Binding requests to a plain STUN server, §5.1.1.2, which have
neither requirement).

### 4.4 Role-conflict tiebreaking (§7.2.5.1, §7.3.1.1)

Client side (§7.2.5.1) — on receiving a **487 (Role Conflict)** error:

- Agent sent ICE-CONTROLLED → switch to controlling.
- Agent sent ICE-CONTROLLING → switch to controlled.
- Either way: requeue the pair that triggered it onto the triggered-check
  queue (state → Waiting), and **change the tiebreaker value**.

Server side (§7.3.1.1) — on receiving a request carrying ICE-CONTROLLING
or ICE-CONTROLLED while local role is set:

- Local=controlling, request carries ICE-CONTROLLING: if local tiebreaker
  `>=` the request's value, respond 487 and **keep** local role; if local
  tiebreaker `<` the request's value, **switch** to controlled (no 487).
- Local=controlled, request carries ICE-CONTROLLED: if local tiebreaker
  `>=` the request's value, **switch** to controlling (no 487); if local
  tiebreaker `<` the request's value, respond 487 and **keep** local role.

## 5. Nomination and concluding (§8.1)

- Only the **controlling** agent nominates (§8.1.1): once it picks a
  valid pair to use, it re-runs the check that produced it (via the
  triggered-check queue) **with USE-CANDIDATE set**. This is the *only*
  nomination this spec permits — once a component's pair is nominated,
  the controlling agent MUST NOT nominate a different pair for that same
  component without an ICE restart. (RFC 5245's "aggressive nomination",
  repeated nominations before settling, is explicitly retired — the
  `ice2` option, §10, signals compliance with this spec's single-shot
  rule to a possibly-RFC-5245 peer.)
- On successful nomination (§8.1.2): all other pairs for that component
  are pulled from the checklist and triggered-check queue (an In-Progress
  one is cancelled — no retransmits, no failure on silence, but the agent
  still waits out the transaction timeout for a straggling response).
  Checklist → Completed once every component has a nominated pair.
  ICE session state → Completed once every checklist is Completed.
- Checklist → Failed when some component has no valid pair and every pair
  for it is Failed/Succeeded; session → Failed once every checklist is
  Failed (or the controlling agent chooses to terminate rather than
  proceed without the failed stream).
- §8.3.1: once Completed, an agent SHOULD wait **3 seconds** before
  ceasing to answer checks on non-selected local candidates (covers a
  straggling RFC 5245 aggressive-nomination peer). Server-reflexive
  candidates are never explicitly freed — they simply lapse once
  keepalives stop.

## 6. Timers (§14)

### 6.1 Ta (§14.2)

- **Default 50 ms.** An agent MAY use another value but MUST tell its
  peer during session establishment if it wants to; **both sides use the
  higher** of the two proposed values (an agent that doesn't propose
  counts as proposing the default when comparing).
- Regardless of per-agent Ta, the combined rate across every ICE agent
  sharing an implementation MUST NOT exceed one transaction per **5 ms**
  (i.e. a de facto global floor, even if that means throttling below each
  agent's own Ta).

### 6.2 RTO (§14.3)

Two different formulas depending on phase:

```
Gathering phase:      RTO = MAX(500ms, Ta * Num-Of-Cands)
                      Num-Of-Cands = number of srflx + relay candidates
                      being gathered

Connectivity checks:  RTO = MAX(500ms, Ta * N * (Num-Waiting + Num-In-Progress))
                      N = total checks to be performed
                      Num-Waiting / Num-In-Progress = live counts in those
                      states across the whole checklist set
```

RTO is **recomputed per transaction** as those counts change — it is not
a fixed value for the session. `500ms` is a hard floor regardless of
formula ("agents MUST NOT use an RTO value smaller than 500 ms").

## 7. Keepalives (§11)

- MUST send a keepalive per data-carrying candidate pair if nothing has
  been sent on it in the last **Tr** seconds. Default Tr **15 seconds**;
  MAY use a larger value, MUST NOT use smaller.
- Once selected pairs exist, keepalives go **only** on those pairs.
- Mechanism: STUN Binding **Indication** (not a request/response
  transaction) — MUST NOT carry any authentication, SHOULD carry
  FINGERPRINT, SHOULD carry nothing else. An agent MUST still be ready to
  receive an actual connectivity check on that same pair, not just an
  indication.

## 8. Data handling (§12)

- MAY send data on any valid pair before selected pairs exist for the
  stream; once selected pairs exist, MUST restrict sending to those.
- Sends go from the local candidate's **base**, to the remote candidate
  (through the TURN server for a relayed local candidate).
- §12.2: MUST NOT treat a source-address change on an inbound RTP/RTCP
  stream as an RFC 3550 §8.2 SSRC collision if the agent can otherwise
  tell (via the STUN exchange) that it's still the same peer switching
  candidates.

## 9. Cross-check against `transport.rs::MediaTransport::new`

```rust
let agent_config = AgentConfig {
    local_ufrag: config.local_ice_ufrag.clone(),
    local_pwd: config.local_ice_pwd.clone(),
    is_controlling: config.is_controlling,
    multicast_dns_mode: MulticastDnsMode::Disabled,
    candidate_types: vec![CandidateType::Host, CandidateType::ServerReflexive],
    ..Default::default()
};
```

- `is_controlling` is caller-supplied (from the WHIP/WHEP offerer/answerer
  role) rather than derived from "who initiated" internally — reasonable,
  since §6.1.1's role rule ultimately reduces to "the offerer/initiator is
  controlling," which the caller already knows from which side of
  WHIP/WHEP it's running.
- `candidate_types: [Host, ServerReflexive]` — **no Relayed (TURN)**, and
  correctly **no PeerReflexive** in this list (§5.1.1 is explicit that
  peer-reflexive candidates are never among the ones an agent gathers up
  front; they only arise from `rtc-ice`'s own connectivity-check
  processing). This matches the crate's own module doc, which says TURN
  relay is out of scope for this cut — a documented scope limitation, not
  a spec violation.
- Host candidate construction (`CandidateHostConfig` → `component: 1`)
  and the server-reflexive gather (`StunGather`, feeding
  `add_server_reflexive_candidate` on resolution) both match §5.1.1.1/
  §5.1.1.2's base/component rules: base of the host candidate is itself;
  base of the resulting server-reflexive candidate is set to
  `self.local_addr` (the host candidate it was gathered from), matching
  "base of a server-reflexive candidate = the host candidate the request
  was sent from."
- `ice.start_connectivity_checks(is_controlling, remote_ufrag, remote_pwd)`
  is called unconditionally right after adding the host candidate and
  before the DTLS/SRTP setup below it — consistent with §3's overall
  ordering (ICE connects first; DTLS-SRTP's keying only starts once a
  candidate pair is up), though the actual checklist/pairing/priority
  logic that follows this call is entirely `rtc-ice`'s, not checked line
  by line here since it isn't this repository's code.

**No discrepancy found** in what `transport.rs` itself configures — the
scope gaps (no TURN/relayed candidates, no lite-agent path, no ICE
restart) are all pre-existing, self-documented cut boundaries rather than
mismatches with what RFC 8445 requires of a full implementation that
*does* claim to support those features.
