# Copy Protection Resource

_Source: ETSI TS 101 699 V1.1.1 §6.6, Tables 69–73 (PDF pp. 61–63), render-verified_

This resource (with **resource_identifier `0x00041ii1`**, resource type `1`, where `ii` = Module ID) is included in hosts which support copy protection — a means of controlling content outputs from a host (audio, video and/or data) to allow or disallow recording or copying of the content. It provides a generic means of communicating with the copy protection function via a generalized set of objects, but the detailed content of each object is specific to the particular copy protection system(s) implemented.

The resource consists of four objects:

- `CP_query` — queries information and status of the resource; the reply is returned in `CP_reply`.
- `CP_reply` — reply to `CP_query`.
- `CP_command` — sends data to the resource.
- `CP_response` — sends data from the resource.

The first pair (`CP_query`/`CP_reply`) are specified with standard queries and replies. The second pair (`CP_command`/`CP_response`) just pass data opaquely between application and resource, with the specific format and semantics of the data defined by the particular copy protection control mechanism implemented in the host.

> **Note on resource_identifier and apdu_tags.** The resource ID is `0x00041ii1` where `ii` is the Module ID (§6.6.1.1: e.g. Module ID 3 → resource ID `0x000410C1`). Per Table 87 the objects are CP_query `0x9F8000`, CP_reply `0x9F8001`, CP_command `0x9F8002`, CP_response `0x9F8003` — matching the tag values cited in the syntax tables below. Note these overlap numerically with the Application MMI tags because both are scoped per-resource. The exact value read from each table header is given per-table.

## §6.6.1 Copy protection system instance management (pp. 61–62)

A host may contain more than one copy protection system (e.g. more than one technology to protect its output; additionally optional interfaces such as a digital recording interface implemented as a module could provide copy protection features). The **instance field** of the resource ID (the `ii` octet) is used to differentiate each copy protection system.

- **§6.6.1.1 Module provided systems** — where a module provides a copy protection system, the module's Module ID shall be used when generating its copy protection resource ID. Example: if its Module ID is 3 then the resource ID presented is `0x000410C1`.
- **§6.6.1.2 Host provided systems** — where a host provides one or more copy protection systems it shall generate resource IDs for each system, setting the instance field of the resource ID to avoid contention with any module provided systems.
- **§6.6.1.3 Application use of copy protection systems** — the set of copy protection resources apparent to an application is the combination of host- and module-provided systems. An application (e.g. a CA system) that requires to control copy protection opens a session to each resource (concurrently or sequentially) and interrogates the resource to determine the system it provides. Applications shall only open sessions to copy protection systems when controlling delivery of a service.

§6.6.1 carries no syntax table — prose only.

## §6.6.2 Copy protection system ID management (p. 62)

The `CopyProtectionID` field contains a value unique to a particular type of copy control mechanism used. This shall be a `company_id` allocated by the IEEE.

§6.6.2 carries no syntax table — prose only.

## §6.6.3 Minimum repetition interval (p. 62)

Copy protection systems shall not require communication between the module and the host more than once per second.

§6.6.3 carries no syntax table — prose only.

## Table 69 — Copy protection query syntax (CP_query) (p. 62)

apdu_tag `CopyProtectionQueryTag` = `0x9F 80 00`, Direction app `--->` host (asks for the current status of the copy protection resource).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cp_query() {` | | |
| &nbsp;&nbsp;CopyProtectionQueryTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;CopyProtectionID | 24 | uimsbf |
| `}` | | |

Field notes:
- `CopyProtectionQueryTag` — 24 bit integer with value `0x9F8000` identifies this message.
- `CopyProtectionID` — 24 bit value identifying the copy protection system that is to be interrogated.

## Table 70 — Copy protection reply syntax (CP_reply) (p. 62)

apdu_tag `CPReplyTag` = `0x9F 80 01`, Direction host `--->` app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cp_reply() {` | | |
| &nbsp;&nbsp;CPReplyTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;CopyProtectionID | 24 | uimsbf |
| &nbsp;&nbsp;Status | 8 | uimsbf |
| `}` | | |

Field notes:
- `CPReplyTag` — 24 bit integer with value `0x9F8001` identifies this message.
- `CopyProtectionID` — as above. This field contains the true ID value for the copy protection mechanism implemented by the resource, even when the status reply is `ID mismatch`.
- `Status` — 8 bit status value; see Table 71.

### Table 71 — Status values (p. 63)

| status | status value |
|--------|--------------|
| Copy Protection Inactive | `01` |
| Copy Protection Active | `02` |
| ID mismatch | `FF` |
| reserved | other values |

## Table 72 — Copy protection command syntax (CP_command) (p. 63)

apdu_tag `CPCommandTag` = `0x9F 80 02`, Direction app `--->` host (sends data to the resource).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cp_command () {` | | |
| &nbsp;&nbsp;CPCommandTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;CopyProtectionID | 24 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CPCommandByte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `CPCommandTag` — 24 bit integer with value `0x9F8002` identifies this message.
- `CopyProtectionID` — as defined above.
- `CPCommandByte` — bytes forming a command message from the application to the resource. **Opaque CP-system-specific bytes** — the coding of this message is specific to the copy control technology.

## Table 73 — Copy protection response syntax (CP_response) (p. 63)

apdu_tag `CPResponseTag` = `0x9F 80 03`, Direction host `--->` app (sends data from the resource). Identical to `CP_command` except for the tag value.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cp_response () {` | | |
| &nbsp;&nbsp;CPResponseTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;CopyProtectionID | 24 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;cp_response_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `CPResponseTag` — 24 bit integer with value `0x9F8003` identifies this message (confirmed by the per-field prose on p. 64: "CPResponseTag — This 24 bit integer with value 0x9F8003 identifies this message"; consistent with Table 87 and §6.6.5 "These objects are identical except for the tag value").
- `CopyProtectionID` — as defined above.
- `cp_response_byte` — bytes forming a response message from the resource to the application. **Opaque CP-system-specific bytes** — coding specific to the copy control technology.
