# broadcast-codec

The codec seam for `rust-broadcast`: raw audio/video frame types, two sans-IO
traits an outside codec plugs into, and the registry that chooses between them.

**This crate holds no codec.** It defines the vocabulary a decoder or encoder
adapter exchanges with the rest of the workspace:

- [`CodecId`] and [`CodecParameters`] say what a stream is and what a decoder
  needs before its first packet; [`CodecProfile`] names the variant of the
  codec a stream is in, with a typed key (`h264/high10`, `aac/lc`) every side
  shares.
- [`Packet`] is one coded access unit; [`Frame`] is one decoded audio or video
  frame. Video frames carry planar, semi-planar or packed storage at 8, 10 or
  12 bits, plus colour description, static HDR metadata and opaque ITU-T T.35
  payloads.
- [`Decoder`] and [`Encoder`] are object-safe, sans-IO traits. A real codec
  library (`symphonia`, `rav1d`, `rav1e`, `openh264`, …) is wrapped in a type
  that implements one of them, without changing this crate or `transmux`.
- [`CodecProvider`] is one adapter offering one or more implementations, and
  [`Registry`] holds any number of providers and picks one per codec, so an
  application links every adapter it likes and no call site names one.

## Contract

The caller drives both traits the same way:

1. `send` one input unit.
2. Call `receive` until it returns `Ok(None)`; that means "send more".
3. Call `finish` at end of input.
4. Keep calling `receive` until it returns `Ok(None)` again; that means
   "drained".

`receive` returning `Ok(None)` is never an error, and a decoder or encoder may
hold input for as long as it needs before producing output. `reset` drops all
held state so the next input starts a new stream.

Every public struct has private fields, a constructor or builder, and
accessor methods. Every public struct and enum is `#[non_exhaustive]`.

The API is **0.x and expected to change** while real codecs are brought through
it. Nothing here participates in the workspace's DVB lockstep; the crate is
unpublished.

## Providers and the registry

A [`CodecProvider`] is one adapter: a name, a [`ProviderInfo`] saying who it
is and under what terms, the [`CodecDescriptor`]s it offers, and a way to build
a [`Decoder`] or [`Encoder`] for one of them. A [`Registry`] is a value the
application builds from those providers. There is no static registry, no
link-time self-registration and no initialisation-time registration hook: the
application decides what it links and passes the registry where it is needed.

A descriptor also states the [`CodecProfile`]s its implementation handles. A
profile is a typed key both sides share (`h264/high10`, `aac/lc`), so a
provider's own spelling never has to be matched against a stream's; an empty
list means "not stated", not "all". [`CodecProvider::profiles`] reports one
provider's profiles for a codec and direction, and [`Registry::profiles`]
attributes each profile to the providers that state it.

```rust
use std::num::NonZeroU32;

use broadcast_codec::{
    CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProvider, Decoder, Direction,
    Encoder, EncoderConfig, Preference, ProviderInfo, Registry, Timescale,
};

struct Alpha;

impl CodecProvider for Alpha {
    fn name(&self) -> &str {
        "alpha"
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new("alpha", "0.1.0")
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        vec![CodecDescriptor::new(
            "alpha",
            "aac_alpha",
            CodecId::Aac,
            Direction::Decode,
        )]
    }

    fn make_decoder(&self, _: &str, _: &CodecParameters) -> Result<Box<dyn Decoder>, CodecError> {
        Err(CodecError::unsupported("this example builds no decoder"))
    }

    fn make_encoder(&self, _: &str, _: &EncoderConfig) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported("this example builds no encoder"))
    }
}

fn build() -> Result<Registry, Box<dyn std::error::Error>> {
    let registry = Registry::new()
        .with(Box::new(Alpha))?
        .prefer(Preference::new(CodecId::Aac, "alpha"));
    let rate = NonZeroU32::new(48_000).ok_or("sample rate")?;
    let parameters = CodecParameters::new(CodecId::Aac, Timescale::from_nonzero(rate));
    assert_eq!(registry.candidates(CodecId::Aac, Direction::Decode).len(), 1);
    assert!(registry.decoder(&parameters).is_err());
    Ok(registry)
}
```

## The selection rule

1. Candidates for a codec and a direction are all matching descriptors, in
   provider registration order and, within a provider, in the order it lists
   them. A selector and a preference reorder that list (rules 2 and 3); rule 4
   then removes from it.
2. If a selector ([`Registry::select_with`]) is set, it is given that list and
   its answer is the first candidate; the rest follow in order. A selector that
   returns `None` or an out-of-range index means "no choice", and rule 3 then
   rule 4 decide.
3. If a [`Preference`] matches the codec and direction, its candidate moves to
   the front. A preference naming a provider without an implementation moves
   all of that provider's candidates to the front, keeping their order. A later
   [`Registry::prefer`] for the same codec, profile and direction replaces an
   earlier one. A preference naming a provider or implementation that is not
   registered is an error from [`Registry::decoder`] and [`Registry::encoder`]
   ([`CodecError::Unsupported`] naming it), never silently ignored; so is one
   naming a registered provider that offers nothing for that codec and
   direction. A preference built with [`Preference::with_direction`] governs
   that direction only and outranks a preference for the same codec that names
   both; a preference built with [`Preference::with_profile`] governs streams
   of that profile only and outranks one for the bare codec.
4. When the stream's [`CodecParameters::profile`] (or the encoder
   configuration's [`EncoderConfig::profile`]) is `Some(p)` and a candidate's
   descriptor states a non-empty profile list that does not contain `p`, that
   candidate is removed. A candidate that states no profiles stays, and a
   stream that states no profile removes nothing. This holds under
   [`Fallback::Strict`] too: it is not a retry, it is not offering a stream to
   an implementation that states it cannot take it. When every candidate is
   removed, [`Registry::decoder`] and [`Registry::encoder`] return
   [`CodecError::Unsupported`] naming the codec, the profile and each removed
   candidate; a preference or a selector that picks a removed candidate is an
   error naming it, never a silent switch to another. Rule 1's list is what
   [`Registry::candidates`] reports;
   [`Registry::candidates_for`] and [`Registry::encoder_candidates_for`] apply
   this clause, and [`Registry::rejected_for`] and
   [`Registry::encoder_rejected_for`] report what was removed and why.
5. With no selector and no preference, registration order decides. The library
   ranks nothing.
6. [`Fallback::Strict`] (the default) tries only the first candidate and
   returns its error. [`Fallback::NextCandidate`] skips a candidate that answers
   [`CodecError::Unsupported`] or [`CodecError::Refused`] and tries the next
   one, while [`CodecError::Failed`] stops at once; when every candidate is
   skipped, the error lists what was tried.
7. No candidate at all is [`CodecError::Unsupported`] naming the codec and
   direction.

## Preferences as text

A preference has a text form, so a command-line flag or a JSON string can carry
it. [`Preference`] implements [`FromStr`](core::str::FromStr) and
[`Display`](core::fmt::Display), and [`parse_preferences`] reads a
comma-separated list:

| Text | Preference |
| --- | --- |
| `aac=oxideav` | AAC, either direction, provider `oxideav` |
| `h264=libav:h264_videotoolbox` | H.264, either direction, provider `libav`, implementation `h264_videotoolbox` |
| `dec:aac=oxideav` | AAC, decode only |
| `enc:aac=libav` | AAC, encode only |
| `h264/high10=libav` | H.264 High 10, either direction |
| `dec:aac/he=oxideav:fdk` | AAC High Efficiency, decode only, implementation `fdk` |
| `other:00000007=alpha` | the `CodecId::Other(7)` format, either direction |

A preference with a profile applies only to streams of that profile, and a
stream of that profile uses it in preference to a preference for the bare
codec. A profile token that names no profile of the codec is a
[`PreferenceError`] naming it.

The codec token is [`CodecId::name`] read back by [`CodecId::from_name`]; case,
spaces, `.`, `-` and `_` are ignored, so `h264` and `H.264` name the same codec.
A codec token may be followed by `/` and a profile token: [`CodecProfile::token`]
writes that pair and [`CodecProfile::from_token`] reads it back, and it is
written without the codec token in the inventory's profiles column. A profile
the enum does not name uses its eight hex digits after the codec's `other:` tag
(`other:00000007/deadbeef`).
A codec this enum does not name uses `other:<8 hex digits>`
(`other:00000007`), the tag being a caller-chosen format identifier.
[`Display`](core::fmt::Display) writes the compact lower-case spelling
(`h264=libav`, or `h264/high10=libav` for a profiled preference), so a printed
preference parses back to the preference it came from. A token that names no
codec, an empty provider or implementation, a character a name may not contain
and an unknown direction prefix are each a [`PreferenceError`] naming the token
it refused.

## The inventory

[`Registry::providers`] reports each registered provider's [`ProviderInfo`] in
registration order: its name, the adapter's own version, the underlying library
and version when there is one, and that library's licence when the adapter can
know it. `licence` is information for whoever distributes the binary, not legal
advice: an adapter reports what the library itself reports or what its manifest
states, and never guesses, so `None` means "not known".

[`Registry::inventory`] adds one [`InventoryEntry`] per descriptor, carrying the
descriptor, its provider's information and a [`Probe`]. Listed means compiled
in; available means it opens on this machine, which a hardware codec may not.
The registry never probes unless asked (`inventory(true)`), because opening a
hardware codec has a cost; `inventory(false)` reports [`Availability::Unknown`]
for every entry. [`Probe::reason`] carries the provider's own words when it is
unavailable.

[`Display`](core::fmt::Display) for [`InventoryEntry`] writes one line: codec
token, direction, provider, implementation, `hw`/`sw`, `lossless` when true, the
profile tokens joined by `/`, availability and licence. [`render_inventory`]
renders a whole slice as a plain table whose column widths come from the
entries, with no terminal assumptions and no colour. The profiles column leaves
out the codec half of each profile token, because the row already names the
codec:

```text
codec  direction  provider  implementation  kind  lossless  profiles   availability  licence
h264   decode     alpha     h264            sw    -         high/main  available     MIT
aac    encode     beta      aac_hw          hw    -         -          unavailable   -
```

## Encoder options

The typed fields of an [`EncoderConfig`] are what every encoder needs. Anything
provider-specific goes in [`EncoderConfig::options`], in order, and a provider
that does not recognise a key returns [`CodecError::Refused`] naming that key.
A provider never ignores an unknown key: silently dropping a setting encodes
something other than what the caller asked for.

## Features

- `std` (default) — forwards `std` to `broadcast-core`. The crate is
  `#![no_std]` plus `alloc` either way.
