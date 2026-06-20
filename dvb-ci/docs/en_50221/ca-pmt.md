# CA PMT object (ca_pmt)

_Source: EN 50221 §8.4.3.4, Table 25 + value tables (PDF pp. 30-31), render-verified_

apdu_tag `Tca_pmt` = `9F 80 32`, Resource = CA Support, Direction host `--->` app.

The CA PMT object is a table extracted from the Programme Map Table (PMT) in the PSI
information (see ISO/IEC 13818-1 §2.4.4.8 and §2.4.4.9) by the host and sent to the
application. It contains all access control information allowing the application to
filter the ECMs itself and to make itself the correct assignment of an ECM stream
with a scrambled component.

## Table 25 — CA PMT object coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_pmt () {` | | |
| &nbsp;&nbsp;ca_pmt_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;ca_pmt_list_management | 8 | uimsbf |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;program_info_length | 12 | uimsbf |
| &nbsp;&nbsp;`if (program_info_length != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;ca_pmt_cmd_id&nbsp;&nbsp;/* at program level */ | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CA_descriptor()&nbsp;&nbsp;/* CA descriptor at programme level */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;stream_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;elementary_PID&nbsp;&nbsp;/* elementary stream PID */ | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ES_info_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (ES_info_length != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ca_pmt_cmd_id&nbsp;&nbsp;/* at ES level */ | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CA_descriptor()&nbsp;&nbsp;/* CA descriptor at elementary stream level */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## ca_pmt_list_management values (Table, p. 31)

| ca_pmt_list_management | value |
|------------------------|-------|
| more     | `00` |
| first    | `01` |
| last     | `02` |
| only     | `03` |
| add      | `04` |
| update   | `05` |
| reserved | other values |

Semantics:
- `first` — first of a new list of more than one CA PMT object; all previously
  selected programmes are replaced.
- `more` — neither first nor last of the list.
- `last` — last CA PMT object of the list.
- `only` — the list is made of a single CA PMT.
- `add` — a new programme selected by the user, all previous selections retained.
  If `add` is received for an already existing programme its action is identical to
  `update`.
- `update` — the CA PMT of a programme already in the list is sent again because
  the version_number or current_next_indicator changed. List management commands act
  only at programme level; ES-level changes within an existing programme must be
  signalled by an `update` with the complete ES list re-sent.

## ca_pmt_cmd_id values (Table, p. 31)

| ca_pmt_cmd_id | value |
|---------------|-------|
| ok_descrambling | `01` |
| ok_mmi          | `02` |
| query           | `03` |
| not_selected    | `04` |
| RFU             | other values |

Semantics:
- `ok_descrambling` — host expects no answer; the application may start descrambling
  or an MMI dialogue immediately.
- `ok_mmi` — application may start an MMI dialogue but shall not start descrambling
  before reception of a new CA PMT with `ca_pmt_cmd_id` = `ok_descrambling`. The host
  guarantees an MMI session can be opened.
- `query` — host expects a CA PMT Reply; application not allowed to start
  descrambling or MMI before a new CA PMT with `ok_descrambling`/`ok_mmi`.
- `not_selected` — host no longer requires that CA application to descramble the
  service; the application shall close any MMI dialogue it has opened.

## Field notes

- The CA PMT contains all CA_descriptors of the selected programme. If several
  programmes are selected, several CA PMT objects are sent. Only CA_descriptors are
  present; all other descriptors are removed from the PMT by the host.
- `CA_descriptor()` is the one defined by ISO/IEC 13818-1 §2.6.16.
- `program_number`, `version_number`, `current_next_indicator`, `stream_type`,
  `elementary_PID`, etc. are as defined in ISO/IEC 13818-1 §2.4.4.8 / §2.4.4.9.
- The CA_descriptor after `current_next_indicator` is at programme level (valid for
  all components). ES-level CA_descriptors apply to that ES only. If both exist for an
  ES, only the ES-level CA_descriptor is taken into account.
- The host re-sends a new CA PMT (or list) when: user selects another programme; a
  'tune' command selects another service; the version_number changes; the
  current_next_indicator changes.
