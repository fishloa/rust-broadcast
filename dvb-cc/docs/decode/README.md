# dvb-cc — caption DECODE docs

This directory grounds the **CEA-708 / CEA-608 caption DECODE model** (the meaning
of the caption byte stream), as distinct from the **carriage** model already
implemented in `dvb-cc` (the `cc_data()` extraction + 608/708 demux from ETSI TS
101 154 Table B.9 — see [`../ts_101_154/b9-cc-data.md`](../ts_101_154/b9-cc-data.md)).

Caption decode folds **into the existing `dvb-cc` crate** — it is **not** a
separate crate. `dvb-cc` already produces typed caption triplets and splits 608
(`cc_type` 0/1) from 708 (`cc_type` 2/3); the decode layer interprets those bytes.

## Source posture

| Layer | Source | Vendorable? |
|---|---|---|
| Caption carriage (`cc_data()`) | ETSI TS 101 154 Table B.9 | already vendored (`../ts_101_154/`) |
| ANC carriage (SMPTE 2038 / IP) | **RFC 8331** (IETF, freely usable) | yes — RFC |
| **Decoder conformance + 708 semantic model** | **47 CFR §79.102** (US Gov **public domain**) | **yes — fully vendorable** → [`cea708-conformance.md`](cea708-conformance.md) |
| 708 bit-level syntax (DTVCC packet / service block / C0/C1/G0/G1/G2/C2/C3 opcode bytes + preset tables) | **ANSI/CTA-708-E** (copyright CTA; cart-gated) | **NOT yet vendored** — see Gaps in the conformance doc |
| 608 bit-level syntax (line-21 control codes + PAC byte values, colour/style codes) | **ANSI/CTA-608-E** (copyright CTA; cart-gated) | **NOT yet vendored** |

Key distinction: §79.102 is **US-Government public domain** (no copyright), so its
text — the decoder requirements *and* the 708 semantic model (services, code-space
mandate, window/pen/colour/opacity/edge enumerations, coordinate limits) — is fully
quotable and vendorable here. The CTA standards (708-E / 608-E), which carry the
exact per-byte opcode/packet/PAC encodings, are copyright and not yet in `specs/`.

## Files

- [`cea708-conformance.md`](cea708-conformance.md) — the §79.102 conformance /
  semantic model (paragraphs (a)–(t)), plus a precise **Gaps — needs CTA-708-E**
  list separating what is public-domain-grounded from what awaits the CTA spec.
