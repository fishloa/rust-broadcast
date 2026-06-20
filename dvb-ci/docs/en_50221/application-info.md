# Application Information objects (application_info_enq / application_info / enter_menu)

_Source: EN 50221 §8.4.2, Tables 20-22 (PDF pp. 27-28), render-verified_

The Application Information resource (resource_identifier `00020041`) enables
applications to give the host a standard set of information about themselves. Provided
only by the host, no session limit. All applications create a session as soon as they
complete their Profile Enquiry phase; the host sends an Application Info Enquiry, the
application returns an Application Info object. The host may later signal the
application to create an MMI session at its top-level menu entry point via the Enter
Menu object.

## Table 20 — Application Info Enquiry object coding

apdu_tag `Tapplication_info_enq` = `9F 80 20`, Direction host `--->` app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `application_info_enq () {` | | |
| &nbsp;&nbsp;application_info_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

## Table 21 — Application Info object coding

apdu_tag `Tapplication_info` = `9F 80 21`, Direction app `<---` host.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `application_info () {` | | |
| &nbsp;&nbsp;application_info_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;application_type | 8 | uimsbf |
| &nbsp;&nbsp;application_manufacturer | 16 | uimsbf |
| &nbsp;&nbsp;manufacturer_code | 16 | uimsbf |
| &nbsp;&nbsp;menu_string_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<menu_string_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;text_char | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

### application_type values (Table, p. 28)

| application_type | value |
|------------------|-------|
| Conditional_Access        | `01` |
| Electronic_Programme_Guide | `02` |
| reserved | other values |

Field notes:
- `application_manufacturer` — values derived from the CA System ID values defined in
  reference [5] (ETSI TS 101 162).
- `manufacturer_code` — content defined by each manufacturer as he wishes.
- `menu_string_length` / `text_char` — the title of the top-level menu entry,
  followed by a sequence of characters. Text is coded using the character sets and
  methods described in reference [4] (ETSI EN 300 468 Annex A).

## Table 22 — Enter Menu object coding

apdu_tag `Tenter_menu` = `9F 80 22`, Direction host `--->` app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `enter_menu () {` | | |
| &nbsp;&nbsp;enter_menu_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

When the application receives the Enter Menu object it shall create an MMI session
(see §8.6) and display its top-level menu.
