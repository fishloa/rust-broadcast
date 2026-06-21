# Power manager resource

_Source: ETSI TS 101 699 V1.1.1 §6.3, Tables 52–55 (PDF pp. 51–52), render-verified_

The Power manager resource, with ID `0x00220041`, is a module-provided resource
that allows a module to indicate to the host that it is engaged in a task that
should be allowed to complete (§6.3). When one or more modules present the Power
manager resource, the host may interrogate each instance of this resource before
deactivating the power supply to the modules; if any module is busy the
deactivation shall be postponed.

Modules shall continue to operate after they have indicated it is OK for the
host to shut down (e.g. a CA module shall continue to descramble), until
explicitly stopped by the host. If there is session traffic after a module has
indicated it is OK to shut down, the host shall re-interrogate the module before
shutting down.

## Table 52 — Activation status state change request object (§6.3.1, p. 51)

apdu_tag `activation_status_change_request_tag` = `0x9F 80 00`, Direction H → M (host → module).

The Activation state change request object from the host to the module "asks"
the module if it is "occupied" with a task that should be allowed to complete
before powering-down the host. Hosts should not send Activation state change
requests to a module more often than once each minute.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `activation_state_change_request() {` | | |
| &nbsp;&nbsp;activation_status_change_request_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;activation_state | 4 | bslbf |
| &nbsp;&nbsp;`}` | | |

Field notes:
- `activation_status_change_request_tag` — this 24 bit field with value `0x9F8000` identifies this message.
- `reserved` — these 4 bits are reserved for future use and shall be set to '0'.
- `activation_state` — this value identifies the requested new activation state (see Table 53).

### Table 53 — Activation state request values (p. 51)

| activation state | requested power mode |
|------------------|----------------------|
| 0      | Standby-passive (note) |
| 1 - 15 | Reserved for future use |

NOTE: Corresponds to the EACEM defined power mode "Standby-passive".

## Table 54 — Activation status change reply object (§6.3.2, p. 52)

apdu_tag `activation_status_change_ack_tag` = `0x9F 80 01`, Direction M → H (module → host).

The Activation state change acknowledge object is sent in response to an
Activation state change request object. It provides an opportunity for the
module to indicate that it is performing a task. If any module provides this
indication the host shall defer changing the activation state (defer the
shutdown). If a module does not reply within 1 second of an Activation state
change request, the host can assume the module assents to the state change.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `activation_state_change_ack() {` | | |
| &nbsp;&nbsp;activation_status_change_ack_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reply_code | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |

Field notes:
- `activation_status_change_ack_tag` — this 24 bit field with value `0x9F8001` identifies this message.
- `reply_code` — this value identifies the module's response to the requested state change (see Table 55).

### Table 55 — Activation status change acknowledge values (p. 52)

| reply_code | description |
|------------|-------------|
| 0       | OK to change state |
| 1       | Module busy, don't change state |
| 2 - 255 | Reserved for future use |
