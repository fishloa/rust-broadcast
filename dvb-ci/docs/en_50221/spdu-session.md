# Session Protocol Data Unit (SPDU) framing

_Source: EN 50221 §7.2.4-7.2.7, Tables 4-14 (PDF pp. 19-23), render-verified_

The session layer uses a Session Protocol Data Unit (SPDU) structure to exchange data
at session level, either host-to-module or module-to-host. An SPDU is transported in
the data field of one or several TPDUs.

## Table 4 — SPDU Coding (general structure, p. 19)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `SPDU() {` | | |
| &nbsp;&nbsp;spdu_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length_value; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;session_object_value byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;apdu() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

The SPDU has two parts: a mandatory session header (a one-byte `spdu_tag`, a
`length_field` coding the length of the session object value field, and the session
object value), and a conditional body of variable length containing an integer number
of APDUs of the same session. The length field does NOT include the length of any
following APDUs. Body presence depends on the session header — only the
`session_number` object (tag `90`) is followed by an APDU body.

`length_field` coding is per Table 1 (see `length-field.md`).

## Table 14 — Session Tag Values (spdu_tag, p. 23)

The `spdu_tag` follows ASN.1 rules and is coded in **one byte**.
Direction column: `<---` host to module, `--->` module to host, `<-->` either.

| spdu_tag | tag value (hex) | Primitive/Constructed | Direction (host <-> module) |
|----------|-----------------|-----------------------|------------------------------|
| Topen_session_request    | `91` | P | `<---` |
| Topen_session_response   | `92` | P | `--->` |
| Tcreate_session          | `93` | P | `--->` |
| Tcreate_session_response | `94` | P | `<---` |
| Tclose_session_request   | `95` | P | `<-->` |
| Tclose_session_response  | `96` | P | `<-->` |
| Tsession_number          | `90` | P | `<-->` |

Other values in the ranges `80`-`8F`, `90`-`9F`, `A0`-`AF`, `B0`-`BF` are reserved.

## Session objects

### Table 5 — Open Session Request coding (p. 20)
Issued by the module to the host to request opening a session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `open_session_request () {` | | |
| &nbsp;&nbsp;open_session_request_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 4 | | |
| &nbsp;&nbsp;resource_identifier()&nbsp;&nbsp;/* see 8.2.2 */ | | |
| `}` | | |

### Table 6 — Open Session Response coding (p. 20)
Issued by the host to the module to allocate a session number or reject the request.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `open_session_response () {` | | |
| &nbsp;&nbsp;open_session_response_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 7 | | |
| &nbsp;&nbsp;session_status | 8 | uimsbf |
| &nbsp;&nbsp;resource_identifier() | | |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |

### Table 7 — Open Session Status values (p. 20)

| session status | session_status value (hex) |
|----------------|----------------------------|
| session is opened | `00` |
| session not opened, resource non-existent | `F0` |
| session not opened, resource exists but unavailable | `F1` |
| session not opened, resource exists but version lower than requested | `F2` |
| session not opened, resource busy | `F3` |
| other | reserved |

`session_nb` — session number allocated by the host. Value 0 is reserved. Used for
all subsequent APDU exchanges until the session is closed. When the session could not
be opened (session_status != 0), session_nb has no meaning.

### Table 8 — Create Session coding (p. 21)
Issued by the host to a module providing a resource, to request opening a session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `create_session () {` | | |
| &nbsp;&nbsp;create_session_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 6 | | |
| &nbsp;&nbsp;resource_identifier() | | |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |

### Table 9 — Create Session Response coding (p. 21)
Issued by the module providing a resource, to tell the host whether the session could
be opened.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `create_session_response () {` | | |
| &nbsp;&nbsp;create_session_response_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 7 | | |
| &nbsp;&nbsp;session_status | 8 | uimsbf |
| &nbsp;&nbsp;resource_identifier() | | |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |

`session_status` uses the same values as open_session_response (Table 7). The module
inserts the class and type of the requested resource but with the version number the
module currently supports. `session_nb` equals the session_nb in the create_session
object it replies to.

### Table 10 — Close Session Request coding (p. 22)
Issued by module or host to close a session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `close_session_request () {` | | |
| &nbsp;&nbsp;close_session_request_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |

### Table 11 — Close Session Response coding (p. 22)
Issued by module or host to acknowledge closing the session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `close_session_response () {` | | |
| &nbsp;&nbsp;close_session_response_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 3 | | |
| &nbsp;&nbsp;session_status | 8 | uimsbf |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |

### Table 12 — Close Session Status values (p. 22)

| session status | session_status value (hex) |
|----------------|----------------------------|
| session is closed as required | `00` |
| session_nb in the request is not allocated | `F0` |
| other | reserved |

### Table 13 — Session Number coding (p. 22)
Always precedes an SPDU body containing APDU(s).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `session_number () {` | | |
| &nbsp;&nbsp;session_number_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;session_nb | 16 | uimsbf |
| `}` | | |
