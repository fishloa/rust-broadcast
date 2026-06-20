# CA Info objects (ca_info_enq / ca_info)

_Source: EN 50221 §8.4.3.1-8.4.3.2, Tables 23-24 (PDF p. 29), render-verified_

The Conditional Access Support resource (resource_identifier `00030041`) provides a
set of objects to support CA applications. Provided only by the host, no session
limit. All CA applications create a session to it after completing their Application
Information phase; the host then sends a CA Info Enquiry, the application replies with
a CA Info object. The session is kept open for periodic operation of the CA PMT / CA
PMT Reply protocol.

## Table 23 — CA Info Enquiry object coding

apdu_tag `Tca_info_enq` = `9F 80 30`, Direction host `--->` app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_info_enq () {` | | |
| &nbsp;&nbsp;ca_info_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

## Table 24 — CA Info object coding

apdu_tag `Tca_info` = `9F 80 31`, Direction app `<---` host.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_info () {` | | |
| &nbsp;&nbsp;ca_info_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CA_system_id | 16 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## Field notes

- `CA_system_id` — lists the CA system IDs supported by this application. Values for
  CA System IDs are defined in reference [5] (ETSI TS 101 162, DVB CA system
  identifier allocation).
- CA PMT is sent by the host to one or several connected CA applications to indicate
  which elementary streams are selected by the user and how to find the corresponding
  ECMs. The host may send the CA PMT to all connected CA applications, or preferably
  only to applications supporting the same `CA_system_id` value as given in the
  CA_descriptor of the selected ES.
