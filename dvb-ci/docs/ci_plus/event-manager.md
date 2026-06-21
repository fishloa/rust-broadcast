# Event Manager resource

_Source: ETSI TS 101 699 V1.1.1 §6.4, Tables 56–61 (PDF pp. 53–56), render-verified_

The Event Manager resource allows modules to define events which should be
signalled to the module (§6.4). If the host is in standby-passive mode when the
event is detected, the activation state will be raised to standby-active and the
module will be notified of the event.

It is a Module-ID-derived resource: one resource instance is created per Module
ID, with the Module ID placed in the `resource_instance` field (resource_class =
Event Manager, `resource_type` = 1, `resource_version` = 1). The
resource_identifier is therefore `0x00231ii1` (type 1\*, where `ii` = Module ID)
— e.g. `0x00231041` for module_id 1, `0x00231081` for module_id 2,
`0x002310C1` for module_id 3.

A module with an event pending opens a session to "its" instance of the Event
Manager each time it is activated; through this module-specific session the
Event Manager can send module-specific messages (such as Event notification).

## §6.4.2 Event Manager resources

- **Number of events** — the Event Manager shall provide sufficient resources to retain one timer event for each transport connection provided by the host.
- **Retention of events** — the host associates each timer event with the identity of the module. The scheduled event is retained until either the scheduled event occurs, or the same module requests a new event (which replaces the current one). Hosts should also handle a module reserving a far-future timer event then being removed.

## §6.4.3 Time range

The host shall be able to accept timer events scheduled anywhere in the future
time range that can be encoded by the event request message. (No syntax/value
table in this subclause.)

## §6.4.4 Resource priorities

When the event is requested, resource contentions at the time the event occurs
cannot be predicted. The host is responsible for arbitrating the resource
requirements of the module over other demands on the host. Direct or indirect
use of the host's resources by the consumer shall have priority over demands
from a module. The system design of the module and any associated services are
responsible for tolerating non-availability of resources. (No syntax/value table
in this subclause.)

## §6.4.5 Power-up timing

The time specified by a module is the time at which it requires the host to be
functioning. The host design is responsible for starting the activation process
suitably before the scheduled time. (No syntax/value table in this subclause.)

## §6.4.6 Energy conservation

The host Power manager may interrogate a module to determine if the host can
revert to a low power consumption mode. While the module is performing the task
for which it booked the timer event it may reply "Module busy, don't change
state" in response to an Activation state change request. When a module
completes the task it shall reply "OK to change state" when interrogated (see
§6.3). (No syntax/value table in this subclause.)

## Table 56 — Event request object (§6.4.7, p. 55)

apdu_tag `event_request_tag` = `0x9F 80 00`, Direction M → H (module → host).

The event request is a message sent by a module to the host to request
activation of the host in response to a specified event.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `event_request () {` | | |
| &nbsp;&nbsp;event_request_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;event_type | 8 | bslbf |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;event_desc | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `event_request_tag` — this 24 bit field with value `0x9F8000` identifies this message.
- `event_type` — identifier of the type of event (see Table 57).
- `event_desc` — a block of bytes defining the event. The format of this block depends on the event type (see Table 58). If there are no event_desc bytes this cancels any event of this event type previously booked by this module.

### Table 57 — Coding of event types (p. 55)

| event_type | description |
|------------|-------------|
| 0       | Timer |
| 1 - 255 | Reserved for future use |

### Table 58 — Coding of the event description bytes for each event type (p. 56)

| event_type | Event description bytes | no bits | mnemonic |
|------------|-------------------------|---------|----------|
| 0       | Start time (like DVB SI EIT start_time) | 40 | bslbf |
|         | Duration (like DVB SI EIT duration) | 24 | bslbf |
| 1 - 255 | Reserved for future use | | |

## Table 59 — Event request acknowledge object (§6.4.8, p. 56)

apdu_tag `event_request_ack_tag` = `0x9F 80 01`, Direction H → M (host → module).

> Note: the spec's Table 59 heading reads simply "Event" — the body and field
> notes describe the event request acknowledge (reply) object.

The event request reply message is sent by the host to the module in response to
"Event request".

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `event_request_ack() {` | | |
| &nbsp;&nbsp;event_request_ack_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;event_type | 8 | bslbf |
| &nbsp;&nbsp;reply | 8 | bslbf |
| `}` | | |

Field notes:
- `event_request_ack_tag` — this 24 bit field with value `0x9F8001` identifies this message.
- `event_type` — identifier of the type of event as described in Table 57.
- `reply` — identifier of the type of the reply (see Table 60).

### Table 60 — Definition of event request reply codes (p. 56)

Note: the spec labels this column header `event_type`, but it lists the `reply`
code values described above.

| event_type (reply code) | description |
|-------------------------|-------------|
| 0       | Event booked OK |
| 1       | Event type not supported |
| 2       | Event resources consumed |
| 3 - 255 | Reserved for future use |

## Table 61 — Event notification object (§6.4.9, p. 56)

apdu_tag `event_notification_tag` = `0x9F 80 02`, Direction H → M (host → module).

The event notification message is sent by the host to the module when an event
requested by the module occurs.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `event_notification() {` | | |
| &nbsp;&nbsp;event_notification_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;event_type | 8 | bslbf |
| `}` | | |

Field notes:
- `event_notification_tag` — this 24 bit field with value `0x9F8002` identifies this message.
- `event_type` — identifier of the type of event as described in Table 57.
