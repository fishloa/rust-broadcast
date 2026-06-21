# Application MMI Resource

_Source: ETSI TS 101 699 V1.1.1 §6.5, Tables 62–68 (PDF pp. 57–61), render-verified_

The host provides an **Application MMI** resource with **resource_identifier `0x00410041`**. It allows a module to interact with the user by launching an application on the host's application execution environment — potentially much more flexible than the low/high level MMIs of EN 50221.

The `RequestStart` object from module to host specifies the application domain required to execute the application and the reference to the initial object. If the host supports the domain it requests the initial object via `FileRequest` and launches it; the module delivers the file via `FileAcknowledge`. Subsequent execution leads to further `FileRequest`/`FileAcknowledge` exchanges as execution draws content and further executables from the module.

The resource consists of six objects: `RequestStart`, `RequestStartAck`, `FileRequest` (`FileReq`), `FileAcknowledge` (`FileAck`), `AppAbortRequest` (`AppAbortReq`), and `AppAbortAck`.

> **Note on apdu_tag values.** Per Table 87 the Application MMI objects are numbered RequestStart `0x9F8000` … AppAbortAck `0x9F8005`. The syntax tables in §6.5.2–6.5.7 each cite tag values in that `0x9F8000`–`0x9F8005` range. The exact value read from each table header is given per-table below.

## §6.5.1 Resource Contention (p. 57)

The module is not guaranteed access to the Application MMI resource — e.g. if the user is interacting with a broadcast application, that application has priority. There are cases (e.g. associated with a CA_PMT dialogue) where the module cannot rely on this MMI method and shall be able to provide its function using another MMI method.

Cases where a module **can** rely on opening a session to the Application MMI resource:

- when responding to an `EnterMenu` from the host (the host may need to kill an executing broadcast application — but the user focus is not on the application at the time);
- when responding to a `GetServiceReq` (this is part of a channel change which will kill any broadcast application associated with the service selected by the user).

§6.5.1 carries no syntax table — prose only.

## Table 62 — RequestStart message (p. 58)

apdu_tag `RequestStartTag` = `0x9F 80 00`, Direction app `--->` host (CI application to host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `RequestStart() {` | | |
| &nbsp;&nbsp;RequestStartTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;AppDomainIdentifierLength | 8 | uimsbf |
| &nbsp;&nbsp;InitialObjectLength | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<AppDomainIdentifierLength; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;AppDomainIdentifier | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for (i=0; i<InitialObjectLength; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;InitialObject | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `RequestStartTag` — 24 bit field with value `0x9F8000` identifies this message.
- `AppDomainIdentifierLength` — 8 bit field, length of the string of bytes specifying the application domain.
- `InitialObjectLength` — 8 bit field, length of the string of bytes specifying the initial object.
- `AppDomainIdentifier` — bytes specifying the required application domain in an application-domain-specific way.
- `InitialObject` — bytes specifying the initial object in an application-domain-specific way. The source of the initial object may be the module (in which case `FileRequest` is used to request it) or another file source; the encoding of the file source within `InitialObject` is a subject for application domain specification.

## Table 63 — Request start fail message (RequestStartAck) (p. 59)

apdu_tag `RequestStartAckTag` = `0x9F 80 01`, Direction host `--->` app (host to CI application; sent if the requested application domain is not supported by the host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `RequestStartAck () {` | | |
| &nbsp;&nbsp;RequestStartAckTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;AckCode | 8 | bslbf |
| `}` | | |

Field notes:
- `RequestStartAckTag` — 24 bit field with value `0x9F8001` identifies this message.
- `AckCode` — 8 bit field communicating the response to the `RequestStart`.

### Table 64 — AckCode values (p. 59)

| AckCode | Meaning |
|---------|---------|
| `0x00` | Reserved for future use. |
| `0x01` | OK — the application execution environment will attempt to load and execute the initial object specified in the `RequestStart` message. |
| `0x02` | Wrong API — application domain not supported. |
| `0x03` | API busy — application domain supported but not currently available. |
| `0x04` to `0x7F` | Reserved for future use. |
| `0x80` to `0xFF` | Domain specific API busy — application domain specific responses equivalent to response `0x03` but providing application domain specific information on why the execution environment is busy (or not available for some other reason such as resource contention), when it will become available etc. |

## Table 65 — File request message (FileReq) (p. 59)

apdu_tag `FileReqTag` = `0x9F 80 02`, Direction host `--->` app (host requests the application to deliver the named file).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `FileReq () {` | | |
| &nbsp;&nbsp;FileReqTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;FileNameByte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `FileReqTag` — 24 bit field with value `0x9F8002` identifies this message.
- `FileNameByte` — a byte of the filename requested.

## Table 66 — File request object (FileAcknowledge) (p. 60)

apdu_tag `FileAckTag` = `0x9F 80 03`, Direction app `--->` host (delivers the file requested by `FileRequest` to the host, or indicates an error if the file cannot be delivered).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `FileAck () {` | | |
| &nbsp;&nbsp;FileAckTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;Reserved | 7 | bslbf |
| &nbsp;&nbsp;FileOK | 1 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;FileByte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `FileAckTag` — 24 bit field with value `0x9F8003` identifies this message.
- `Reserved` — these 7 bits are reserved for future use and shall be set to '0'.
- `FileOK` — 1 bit field set to '1' if the file is available and '0' otherwise.
- `FileByte` — a byte of the file requested.

## Table 67 — File request message (AppAbortRequest) (p. 60)

apdu_tag `AppAbortReqTag` = `0x9F 80 04`, Direction host `--->` app **or** app `--->` host (can be sent by either host or module to request termination of the executing application process). The exact semantics are defined by the application domain; e.g. this call allows a process to be killed without releasing the associated MMI session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `AppAbortReq () {` | | |
| &nbsp;&nbsp;AppAbortReqTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;AbortReqCode | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `AppAbortReqTag` — 24 bit field with value `0x9F8004` identifies this message.
- `AbortReqCode` — this octet string provides an application domain specific qualification of the kill request.

## Table 68 — File request object (AppAbortAck) (p. 61)

apdu_tag `AppAbortAckTag` = `0x9F 80 05`, Direction (response to `AppAbortRequest`; allows an application domain specific response to the request for the application abort).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `AppAbortAck () {` | | |
| &nbsp;&nbsp;AppAbortAckTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;AbortAckCode | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `AppAbortAckTag` — 24 bit field with value `0x9F8005` identifies this message.
- `AbortAckCode` — this octet string provides an application domain specific response to the kill request.

> **Note.** Tables 67 and 68 are both captioned "File request object/message" in the spec — this is a copy-paste artefact in TS 101 699 V1.1.1; the captions are reproduced verbatim above. The table contents are the `AppAbortReq`/`AppAbortAck` syntax as transcribed.
