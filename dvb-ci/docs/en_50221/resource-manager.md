# Resource Manager objects (profile_enq / profile / profile_changed)

_Source: EN 50221 §8.4.1, Tables 17-19 (PDF pp. 26-27), render-verified_

The Resource Manager (resource_identifier `00010041`) is a resource provided by the
host. There is only one type in the class and it can support any number of sessions.
It controls the acquisition and provision of resources to all applications via a
symmetrical communication protocol between module and host. Cannot be superseded by a
module-provided resource.

## Table 17 — Profile Enquiry object coding

apdu_tag `Tprofile_enq` = `9F 80 10`, Direction `<-->`.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_enq () {` | | |
| &nbsp;&nbsp;profile_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

The Profile Enquiry object requests the recipient to reply with a list of the
resources it provides in a Profile Reply object.

## Table 18 — Profile Reply object coding

apdu_tag `Tprofile` = `9F 80 11`, Direction `<-->`.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_reply () {` | | |
| &nbsp;&nbsp;profile_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = N*4 | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;resource_identifier() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Sent in response to a profile enquiry; lists the resources the sender can provide.
Each `resource_identifier()` is 4 octets (see `resource-identifier.md`), hence
`length_field = N*4`.

## Table 19 — Profile Changed object coding

apdu_tag `Tprofile_change` = `9F 80 12`, Direction `<-->`.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_changed () {` | | |
| &nbsp;&nbsp;profile_changed_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

Notifies the recipient that a resource has changed. A module typically uses it to
notify the host that the availability status of any of its resources changed. The
host modifies its resource list and, if anything changed, sends a Profile Changed
object on all transport connections.
