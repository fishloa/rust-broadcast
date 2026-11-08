# webrtc-runtime 0.6.0

_Released 2026-11-08._

Breaking-class release (0.5 -> 0.6): WHIP and WHEP now run on one generic client engine and one generic server engine, so the `State` and `Event` enums are shared between the two protocols and gain variants. The public type names and method calls are unchanged.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

The WHIP and WHEP clients were two near-identical state machines, as were the two servers. They are now `session::client::Client<P>` and `session::server::Session<P>`, generic over a sealed `session::Profile` with two implementations, `Whip` and `Whep`. The old names stay as type aliases:

```text
WhipClient   = session::client::Client<Whip>
WhepPlayer   = session::client::Client<Whep>
WhipSession  = session::server::Session<Whip>
WhepSession  = session::server::Session<Whep>
```

Construction and method calls read the same as before, so code that only calls methods needs no change. What can break is code that matches exhaustively on `State` or `Event`, and code that names a direction-specific item by its old path.

### Shared `State` and `Event`

Each role now has one `State` and one `Event` type that both protocols use. `whep::player::State` and `whep::player::Event` are the same types as `whip::client::State` and `whip::client::Event`; `whep::server::State` and `whep::server::Event` are the same types as `whip::server::State` and `whip::server::Event`.

```text
whip::client::State   gains  CounterOffered { session_url }
whip::client::Event   gains  CounterOffer(Vec<u8>)
whip::server::State   gains  CounterOffered { etag }
whip::server::Event   gains  SdpAnswer(Vec<u8>)
```

A WHIP session never enters or emits the WHEP-only variants. The enums are `#[non_exhaustive]`, so a downstream `match` already needed a wildcard arm and keeps compiling; code that listed every WHIP variant and relied on the wildcard being unreachable should handle the new ones explicitly:

```rust
// before
match client.state() {
    State::Idle | State::OfferSent => { /* ... */ }
    State::Established { .. } => { /* ... */ }
    State::Closed => { /* ... */ }
    _ => {}
}

// after: the WHEP-only state is now a named variant of the same type
match client.state() {
    State::Idle | State::OfferSent => { /* ... */ }
    State::CounterOffered { .. } => { /* WHEP only; never reached by a WHIP client */ }
    State::Established { .. } => { /* ... */ }
    State::Closed => { /* ... */ }
    _ => {}
}
```

The same applies to `Event` and to the server enums. Code that matched `whep::player::State` and `whip::client::State` in two separate functions can now share one.

### One content-type module

`whip::content_type` and `whep::content_type` are now one module, `session::content_type`. Import it from there, or keep using the re-exports in `whip` and `whep`.

```text
whip::content_type::X   ->  session::content_type::X
whep::content_type::X   ->  session::content_type::X
```

### What stays direction-specific

`Profile::EGRESS` selects the behaviour that differs between the protocols: the client handling of `406` (counter-offer) and `409` (no publisher), the server handling of `PATCH` without `If-Match` (WHIP answers `428`, WHEP accepts it), and the WHEP-only client methods `counter_offer`, `ack_answer`, `no_publisher`, `answer_counter_offer` and `trickle_ice` (the WHIP client has `flush_candidates`). Calling a WHEP-only method on a WHIP client is rejected the same way it was before.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `webrtc-runtime-v0.6.0`.
