# SRT (draft-sharabayko-srt-01) — §4.3.2 Rendezvous Handshake rules

Curated state-machine rules for a Rendezvous handshake implementation.
Source: `specs/ietf_draft_sharabayko_srt_01.txt`, §4.3.2 "Rendezvous
Handshake", **L2094-2433** (§4.3.2 heading through the end of §4.3.2.2, just
before §4.4 "SRT Buffer Latency" begins at L2434). Line cites below are exact
source lines.

This document is the Rendezvous flow only. The Caller-Listener flow (§4.3.1)
is already implemented in `srt-runtime/src/caller.rs` / `listener.rs`; see
the "Contrast with Caller-Listener" section at the end for what differs.

## Overview and version gate (L2096-2105)

> "The Rendezvous process uses a state machine. It is slightly different
> from UDT Rendezvous handshake [GHG04b], although it is still based on the
> same message request types." (L2096-2098)

- Both parties **start with WAVEAHAND and use the Version value of 5**
  (L2100).
- Legacy Version 4 clients do not look at the Version value; Version 5
  clients can detect Version 5. Parties only continue with the **Version 5
  Rendezvous process when Version is set to 5 for both** — otherwise the
  process continues exclusively per Version 4 rules [GHG04b] (L2101-2105).
  The Version-4 legacy path is out of scope for this note (implementation-
  defined / external reference, not detailed in this draft).

## Cookie contest (role assignment) — L2107-2135

- With Version 5 Rendezvous, **both parties create a cookie** for the
  "cookie contest," which assigns the **Initiator** and **Responder** roles
  (L2107-2109).
- Each party generates a **32-bit cookie** based on host, port, and current
  time with **1-minute accuracy**, scrambled with an **MD5 sum calculation**
  (L2109-2112).
- Cookie values are then compared (L2112).
- **Tie-break rule**: since two sockets can't be bound to the same NIC/port
  and operate independently, identical cookies are "virtually impossible"
  except when an application "connects to itself" (local IP / same bound
  address) (L2114-2119).
  - If cookies are identical (for any reason), **the connection will not be
    made until new, unique cookies are generated** (after a delay of up to
    one minute) (L2119-2122).
  - In the self-connect case, cookies will always be identical, so **the
    connection will never be established** (L2122-2124).
- **Role rule**: "When one party's cookie value is greater than its peer's,
  it wins the cookie contest and becomes Initiator (the other party becomes
  the Responder)." (L2133-2135) — greater cookie wins, unlike a numeric
  "lower wins" convention seen in some other protocols; there is no
  secondary tie-break beyond cookie regeneration.

At this point there are two possible "handshake flows": **serial** and
**parallel** (L2137-2138).

## §4.3.2.1 Serial Handshake Flow (L2140-2269)

Applies when the parties' WAVEAHAND sends interleave such that one party
(here "Alice") receives a WAVEAHAND before sending her own next one, and
replies with CONCLUSION instead — meanwhile the peer ("Bob") never sees
Alice's WAVEAHAND, so Alice's CONCLUSION is the first message Bob receives
from her (L2142-2148).

Message/state sequence (numbered steps per the spec, L2153-2269):

1. **(L2153-2170)** Both parties start in the **waving** state. Alice sends
   a handshake to Bob:
   - Version: `5`
   - Extension field: `0`, Encryption field: advertised `PBKEYLEN`
   - Handshake Type: **WAVEAHAND**
   - SRT Socket ID: Alice's socket ID
   - SYN Cookie: created from host/port + current time

   Alice does not yet know if Bob is Version 4 or 5; a V4 peer would not
   interpret these field values when Handshake Type is WAVEAHAND.

2. **(L2172-2201)** Bob receives Alice's WAVEAHAND, switches to
   **"attention"** state. He now knows Alice's cookie, performs the cookie
   contest: if Bob's cookie > Alice's, Bob becomes **Initiator**, else
   **Responder**. Role resolution here is essential to further processing
   (L2178-2179). Bob then responds with Handshake Type **CONCLUSION**:
   - Version: `5`
   - Extension field: appropriate flags if Initiator, else `0`
   - Encryption field: advertised PBKEYLEN
   - If Bob is Initiator and encryption is on, he uses his own cipher
     family/block size or Alice's advertised one if she sent it
     (L2199-2201).

3. **(L2203-2219)** Alice receives Bob's CONCLUSION. She also runs the
   cookie contest (same outcome), switches to **"fine"** state, and sends
   Handshake Type **CONCLUSION**:
   - Version: `5`
   - Appropriate extension + encryption flags
   - Both parties always send extension flags at this point: **HSREQ** if
     from an Initiator, **HSRSP** if from a Responder (L2213-2215). If the
     Initiator previously received the Responder's advertised cipher
     family/block size in the encryption flags, it becomes the key length
     used for KMREQ key generation sent next (L2215-2219).

4. **(L2221-2249)** Bob receives Alice's CONCLUSION and branches on role:
   - **If Bob is Initiator** (Alice's message contains HSRSP): switches to
     **"connected"** state, sends Alice a message with Handshake Type
     **AGREEMENT** carrying **no SRT extensions** (Extension Flags = `0`)
     (L2224-2230).
   - **If Bob is Responder** (Alice's message contains HSREQ): switches to
     **"initiated"** state, sends Alice a **CONCLUSION** that also contains
     extensions with **HSRSP**, and awaits Alice's confirmation that she is
     connected too (preferably an AGREEMENT message) (L2232-2249).

5. **(L2251-2259)** Alice receives the above, enters **"connected"** state,
   then branches on her role:
   - **If Alice is Initiator** (received CONCLUSION with HSRSP): sends Bob
     Handshake Type **AGREEMENT**.
   - **If Alice is Responder**: the received message has Handshake Type
     **AGREEMENT**; she does nothing in response.

6. **(L2261-2268)** If Bob was Initiator, he is already connected. If Bob
   was Responder, he should receive the AGREEMENT message above, after
   which he switches to **"connected"**. If that UDP packet is lost, Bob
   still enters "connected" once he receives *anything else* from Alice. If
   Bob is going to send in the meantime, **he must keep sending the same
   CONCLUSION** until he gets Alice's confirmation.

## §4.3.2.2 Parallel Handshake Flow (L2270-2432)

Low-probability case: both peers send *and* receive WAVEAHAND at precisely
the same time (L2272-2274). Both parties then follow the same state
sequence Bob followed above, but symmetrically:

```
Waving -> Attention -> Initiated -> Connected
```
(L2280, verbatim)

In Attention, both know each other's cookies and assign roles as in the
serial flow (cookie contest, same "greater wins" rule). Unlike the mostly
request/response serial flow, everything here is **asynchronous**: state
switches purely on receipt of a message with the right extension content.
**The Initiator MUST attach the HSREQ extension; the Responder MUST attach
the HSRSP extension** (L2282-2288).

### (1) Initiator state table (L2301-2334)

1. **Waving**: receives WAVEAHAND → switches to **Attention** → sends
   CONCLUSION + HSREQ.
2. **Attention**: receives CONCLUSION —
   - no extensions → switches to **Initiated**, still sends CONCLUSION +
     HSREQ; or
   - contains HSRSP → switches to **Connected**, sends AGREEMENT.
3. **Initiated**: receives CONCLUSION —
   - no extensions → **REMAINS IN THIS STATE** (spec's emphasis, L2325),
     still sends CONCLUSION + HSREQ; or
   - contains HSRSP → switches to **Connected**, sends AGREEMENT.
4. **Connected**: may receive CONCLUSION and respond with AGREEMENT, but
   normally by now payload packets should already be flowing (L2331-2334).

### (2) Responder state table (L2336-2382)

1. **Waving**: receives WAVEAHAND → switches to **Attention** → sends
   CONCLUSION (no extensions).
2. **Attention**: receives CONCLUSION with HSREQ.
   - If that CONCLUSION carries no extensions, the party SHALL simply send
     the empty CONCLUSION again and remain in this state (L2357-2360).
   - Otherwise switches to **Initiated** and sends CONCLUSION + HSRSP.
3. **Initiated**: receives —
   - CONCLUSION with HSREQ → responds with CONCLUSION + HSRSP, remains in
     this state;
   - AGREEMENT → responds with AGREEMENT, switches to **Connected**;
   - a payload packet → responds with AGREEMENT, switches to **Connected**.
4. **Connected**: not expecting any more handshake messages. AGREEMENT is
   sent only once, or per every final CONCLUSION message received
   (L2377-2381).

### Missing-packet recovery rules (L2383-2432)

Any of the above packets may be lost without the sender ever knowing; the
draft prescribes recovery per case:

1. **(L2387-2389)** If the Responder misses CONCLUSION+HSREQ, it keeps
   sending empty CONCLUSION messages; only on receiving CONCLUSION+HSREQ
   does it respond with CONCLUSION+HSRSP.
2. **(L2391-2395)** If the Initiator misses the CONCLUSION+HSRSP response,
   it keeps sending CONCLUSION+HSREQ. The Responder **MUST always** respond
   with CONCLUSION+HSRSP whenever it gets CONCLUSION+HSREQ, even if already
   processed once before (idempotent re-reply, not "only once").
3. **(L2413-2422)** When the Initiator switches to Connected, it sends
   AGREEMENT — which the Responder may miss. The Initiator may still start
   sending data packets, believing itself connected, unaware the Responder
   hasn't switched yet. To compensate, it is **exceptionally allowed** that
   a Responder in the Initiated state which receives a data packet (or any
   control packet normally only sent between connected parties) **may
   switch to Connected** as if it had received AGREEMENT.
4. **(L2424-2432)** If the Initiator has already switched to Connected, it
   sends no more handshake messages, so the Responder (having possibly
   missed AGREEMENT) does not know to exit connecting state. It therefore
   **continues sending CONCLUSION+HSRSP** until it receives *any* packet
   that causes it to switch to Connected (normally AGREEMENT) — only then
   does the application start transmission.

## Contrast with Caller-Listener flow (§4.3.1, implemented in `caller.rs`/`listener.rs`)

The currently-implemented `srt-runtime` handshake (per `caller.rs`/
`listener.rs` doc comments, citing §4.3.1) differs from Rendezvous in every
structural respect:

| Aspect | Caller-Listener (§4.3.1, implemented) | Rendezvous (§4.3.2, this doc) |
|---|---|---|
| Roles | Fixed a priori: Caller is always the initiating side, Listener is always the responding side (no contest) | Roles (Initiator/Responder) are decided dynamically by the **cookie contest** — either peer can become either role |
| First message | Caller sends **INDUCTION** (Version 4, Encryption 0, SYN Cookie 0) | Both peers send **WAVEAHAND** (Version 5) simultaneously/independently |
| Cookie use | Listener mints one cookie and hands it to the Caller in the INDUCTION response; Caller must echo it back verbatim in CONCLUSION (a flood-protection handshake cookie, `caller.rs` L236, `listener.rs` L214) — **no contest**, just echo-and-verify | Both peers generate their own cookie and **compare** them; the winner (greater value) becomes Initiator |
| Handshake types used | INDUCTION, CONCLUSION only | WAVEAHAND, CONCLUSION, **AGREEMENT** (Rendezvous-only type; not used in §4.3.1 at all) |
| State machine | 3-4 named states (Idle/Induction-sent/Conclusion-sent/Connected in `CallerHandshakeState`; mirrored in `ListenerHandshakeState`) with a single linear exchange | Named states **waving / attention / fine (serial only) / initiated / connected**, with two distinct flows (serial vs. parallel) and explicit idempotent-resend recovery rules for lost packets |
| Symmetry | Asymmetric: Caller and Listener run different code paths | Symmetric: both peers run (conceptually) the same state machine; only the resolved Initiator/Responder role differentiates behavior |

No code changes are implied by this note — it exists to scope a future
Rendezvous implementation and make clear it cannot reuse the Caller/Listener
state machines as-is; the message set (WAVEAHAND/AGREEMENT), the cookie
contest, and the two-flow (serial/parallel) branching are all new.

## Fidelity check — constants, formulas, states, messages verified against source

| Item | Value / rule | Source line(s) |
|---|---|---|
| Rendezvous initial Version | `5` | L2100 |
| Cookie size | 32-bit | L2109-2110 |
| Cookie time granularity | 1 minute accuracy | L2110-2111 |
| Cookie scrambling | MD5 sum | L2111-2112 |
| Role rule | greater cookie value -> Initiator | L2133-2135 |
| Identical-cookie behavior | connection withheld until new cookies generated (up to 1 min delay); self-connect never establishes | L2119-2124 |
| Two flows named | "serial" and "parallel" | L2137-2138 |
| Serial flow states | waving, attention, fine, initiated, connected | L2153, L2172, L2205, L2234/2321 (initiated appears in both flows), L2226/2251 (connected) |
| WAVEAHAND fields | Version 5, Ext field 0, Encryption field PBKEYLEN, Handshake Type WAVEAHAND, SRT Socket ID, SYN Cookie | L2156-2165 |
| Extension tags | HSREQ (Initiator), HSRSP (Responder) | L2213-2215, L2224-2249, L2282-2288 |
| AGREEMENT carries no extensions (serial, Bob-as-Initiator case) | Extension Flags field should be 0 | L2228-2230 |
| Parallel flow state diagram | `Waving -> Attention -> Initiated -> Connected` | L2280 |
| Parallel Initiator MUST | attach HSREQ | L2286-2287 |
| Parallel Responder MUST | attach HSRSP | L2287-2288 |
| Initiated-state "remains" rule (Initiator, no-ext CONCLUSION) | explicit emphasis "REMAINS IN THIS STATE" | L2325 |
| Responder empty-CONCLUSION echo rule | when HSREQ msg carries no extensions | L2357-2360 |
| Recovery rule 2 (Responder MUST always resend HSRSP) | "even if it has already received and interpreted it" | L2391-2395 |
| Recovery rule 3 (Responder may promote to Connected on data/control packet) | "exceptionally allowed" | L2413-2422 |
| Recovery rule 4 (Responder keeps sending CONCLUSION+HSRSP until any packet arrives) | until AGREEMENT (or equivalent) received | L2424-2432 |
| §4.4 boundary (not included) | "SRT Buffer Latency" heading | L2434 |

No values were invented; every constant/state/message name above is quoted
or paraphrased directly from the cited lines. The Version-4-legacy
Rendezvous path (referenced at L2101-2105 via external reference [GHG04b])
is **not detailed in this draft** and is explicitly out of scope here —
flagged rather than guessed.
