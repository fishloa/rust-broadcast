## Table 4.3 — IOP::IOR syntax
_§4.7.3.1, PDF p. 30_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `type_id_length` | 32 | N1 | |
| `type_id_byte` × N1 | 8 each | + | see Table 4.4 (DVB: a 3-char alias + NUL ⇒ N1 = 4) |
| `alignment_gap` × (4−(N1%4)) | 8 each | `0xFF` | **only if** `N1 % 4 ≠ 0` — CDR alignment. Never present for DVB alias type_ids (N1=4). |
| `taggedProfiles_count` | 32 | N2 | ≥ 1; first profile is TAG_BIOP or TAG_LITE_OPTIONS |
| per profile: `profileId_tag` | 32 | + | e.g. TAG_BIOP / TAG_LITE_OPTIONS |
| per profile: `profile_data_length` | 32 | N3 | |
| per profile: `profile_data_byte` × N3 | 8 each | | e.g. a BIOPProfileBody / LiteOptionsProfileBody |

DVB guideline: only alias type_ids are used (so no alignment stuffing). Receivers
must process at least the first profile body; others may be ignored.

