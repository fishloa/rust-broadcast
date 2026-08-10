# ITU-T H.222.0 / ISO/IEC 13818-1 — cited clauses

`mpeg-ts` owns MPEG-2 TS framing here, so the H.222.0 references the workspace
relies on are collected in this directory. H.222.0 is cited **451 times across
10 crates**.

## Where the full text lives — deliberately not here

H.222.0 is a **paywalled ITU-T standard**. The PDF is vendored in the private
submodule (`private/specs/itu_t_h222_0_202308_mpeg2_systems.pdf`) precisely
because it is not redistributable, and the full machine conversion lives beside
it at **`private/docs/h222-0-transport-stream.md`** (pages 39–80) and
**`private/docs/h222-0-ca-descriptor.md`** (page 108).

This repository is public, so it carries only what a citation needs: the short
normative clauses code decisions actually turn on. That matches the existing
`dvb-si/docs/` precedent, which holds extracted syntax tables rather than
reproduced prose.

Maintainers: `git submodule update --init private`, then read
`private/docs/`. Public clones and CI skip the submodule and the dependent
tests skip cleanly.

## How the conversion was produced

With the local `pdf2md` tool, which reads the PDF's embedded text layer and
**diffs every bit/hex/digit token against its own output**, exiting non-zero if
any value was dropped or altered. Both files converted with **exit 0**.

```bash
cd ~/Projects/pdf2md
uv run pdf2md convert <spec.pdf> -o <out.md> --pages 39-80 --engine hybrid --report
```

Use this rather than `pdftotext`, which mangles table structure and has
produced wrong transcriptions in this repo before. Always check the exit code.

## §2.4.3.3 — duplicate packets

The clause `dvb-conformance`, `media-doctor` and `ts-fix` each implement, and
which issue #956 exists because two of them disagreed on. Verbatim
(Rec. ITU-T H.222.0 (08/2023) §2.4.3.3, PDF p. 48):

> In transport streams, duplicate packets may be sent as two, and only two, consecutive transport stream packets of the same PID. The duplicate packets shall have the same continuity_counter value as the original packet and the adaptation_field_control field shall be equal to '01' or '11'. In duplicate packets each byte of the original packet shall be duplicated, with the exception that in the program clock reference fields, if present, a valid value shall be encoded.

Two consequences the code depends on:

- **"each byte of the original packet"**, with the PCR fields the *sole*
  exception — so a payload-only comparison is too lenient. A packet differing
  in `splice_countdown` or OPCR is **not** a legal duplicate.
- **"two, and only two"** consecutive packets — a third repeat is an error
  regardless of byte-identity.

## §2.4.3.3 — continuity_counter

Verbatim (same section, PDF p. 48):

> The continuity_counter in a particular transport stream packet is continuous when it differs by a positive value of one from the continuity_counter value in the previous transport stream packet of the same PID, or when either of the non-incrementing conditions ( adaptation_field_control set to '00' or '10', or duplicate packets as described above) are met. The continuity counter may be discontinuous when the discontinuity_indicator is set to '1' ( refer to 2.4.3.4). In the case of a null packet the value of the continuity_counter is undefined.

## Section index

The full conversion in `private/docs/` covers, with PDF heading pages:

| § | Title | p. |
|---|---|---|
| 2.4.2.2 | System clock frequency | 39 |
| 2.4.2.3 | Input to the transport stream system target decoder | 40 |
| 2.4.2.4 | Buffering | 41 |
| 2.4.3.2 | Transport stream packet layer | 48 |
| 2.4.3.3 | Semantic definition of fields in transport stream packets | 48 |
| 2.4.3.4 | Adaptation field | 50 |
| 2.4.3.5 | Semantic definition of fields in adaptation field | 51 |
| 2.4.3.6 | PES packet | 61 |
| 2.4.3.7 | Semantic definition of fields in PES packet | 64 |
| 2.4.4 | Program-specific information | 74 |
| 2.4.4.3 | Semantics definition of fields in pointer syntax | 76 |
| 2.4.4.5 | Table_id assignments | 77 |
| 2.4.4.8 | Semantic definition of fields in conditional access section | 78 |
| 2.6.16 | Conditional access descriptor | 108 |
