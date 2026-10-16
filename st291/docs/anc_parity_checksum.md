# SMPTE ST 291-1:2011 — word parity and checksum (primary source)

_Source: SMPTE ST 291-1:2011 "Ancillary Data Packet and Space Formatting"
(vendored at `private/specs/smpte_st291-1_2011_anc_packet_space_formatting.pdf`,
text at `private/specs-md/…`). Verified on the rendered PDF pages (PDF p.9,
p.12, p.17, rendered at 80 dpi and read), not only the markdown._

This is the **primary-source** grounding for the rules `src/anc_content.rs`
implements. `anc_packet_291.md` derives the same rules via RFC 8331 §2.1.

## §6.1 DID, §6.2 SDID, §6.4 DBN, §6.5 Data Count (PDF p.9, p.10-12)

Each word is 10 bits: an 8-bit value plus even parity and its inverse:

- `b7..b0` — the 8-bit value
- `b8` — even parity for bits `b7` through `b0`
- `b9` = NOT `b8`

(DID §6.1, SDID §6.2 Type 2 only, DBN §6.4 Type 1 only — it takes the SDID's
position — and DC §6.5, which counts 0..=255 user data words.)

## §6.6 User Data Words (PDF p.12)

10-bit applications: a UDW is the full `b9..b0`. **No parity rule** is stated
for UDWs; they "shall not include protected codes" (§9). 8-bit applications use
`b9..b2`. Parity must therefore never be validated on UDWs.

## §6.7 Checksum Word (PDF p.12)

10 bits: `b8..b0` is the checksum value, `b9` = NOT `b8`. The value is the nine
least significant bits of the sum of the nine least significant bits of DID,
DBN-or-SDID, DC and all UDWs; the sum starts from zero and any end carry is
ignored. (Same as RFC 8331 §2.1.)

## Annex C worked words (PDF p.17, informative, "inclusive of parity bits")

DID: `2FF 1FD 2FC 180 2F5 260 2C0 140 104`; DBN/SDID: `101 200 260 110`;
DC: `230 224 206 108 110` (48, 36, 6, 8, 16 words). All satisfy the §6.1 rule
(e.g. `0xF5` has six 1s -> b8 = 0, b9 = 1 -> `0x2F5`; `0x30` has two ->
`0x230`; `0xFD` has seven -> `0x1FD`). Checksums are omitted in the
Annex, so checksum vectors in tests are hand-computed from §6.7.

> Spec note: the Annex B figure labels the parity bit `p` and its inverse
> `p/` (markdown shows "p/"); it is `b9`. The Annex C DC cell for the
> "marked for deletion" packet is `xxx`.

## Crate mapping

| Rule | API |
|---|---|
| parity of one word | `AncContent::has_valid_parity` |
| DID/SDID/DC parity | `AncContent::verify_parity` -> `Error::ParityMismatch` |
| all checks, per rule | `AncContent::validate` / `AncPacket::validate` -> `ValidationReport` |
| build with parity | `AncContent::with_parity`, `AncContent::build` |
| checksum | `AncContent::compute_checksum` / `verify_checksum` |
