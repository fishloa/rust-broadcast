# DVB-MABR service reporting — ETSI TS 103 769 V1.2.1 clause 11, Annex N

Source: ETSI TS 103 769 V1.2.1 (2024-11). See [`README.md`](README.md) for provenance.

This document transcribes the **service reporting information document** — the JSON
format a Multicast gateway POSTs to the Service reporting capture function at reference
point `RS`. It draws from:

- **Clause 11** (the normative prose describing reporting elements, procedures, and the
  per-service-component metrics table).
- **Annex N** (the complete OpenAPI 3.0.1 YAML schema — discovered during the fidelity
  audit and previously missed by the initial transcription; see [README.md](README.md)
  "Audit history").

The transcription below is based on the OpenAPI YAML (authoritative for types and enum
values), cross-checked against clause 11's prose and property tables, with conflicts
flagged explicitly.

## ⚠️ Flagged spec-internal conflict: three incompatible `object-delivery-status` enumerations

TS 103 769 V1.2.1 carries **three** mutually inconsistent versions of the
media-object delivery status enumeration. They are listed here side by side so an
implementer can see the shape of the conflict — none can be taken alone:

| Source | Location | Values listed |
|---|---|---|
| Clause 11.1.2.2, Table 11.1.2.2-1 | Normative body (page 89) | 10 values: `cache-hit-m`, `cache-hit-a`, `cache-hit-mr`, `cache-miss-expired`, `cache-miss-incomplete`, `cache-miss-filter`, `cache-miss-nodata-s`, `cache-miss-nodata-m`, `cache-miss-nodata-j`, `cache-miss-nodata-o` |
| Annex N, `MABR_Event.object-delivery-status` | Normative OpenAPI schema (page 180) | 9 values: `cache-hit-m`, `cache-hit-a`, `cache-miss-incomplete`, `cache-miss-filter`, `cache-miss-nodata-s`, `cache-miss-nodata-m`, `cache-miss-nodata-j`, `cache-miss-nodata-o`, `cache-miss-timeshift` |
| Annex N, `MABR_ObjectDeliveryStatus` | Normative OpenAPI schema (page 183) | 10 values (9 unique): `cache-hit-m`, `cache-hit-a`, `cache-miss-expired`, `cache-miss-incomplete`, `cache-miss-filter`, `cache-miss-nodata-s`, `cache-miss-nodata-m`, `cache-miss-nodata-j`, `cache-miss-nodata-o`, `cache-miss-expired` (duplicated) |

**Key observations:**

1. No two lists are identical. Clause 11.1.2.2 has `cache-hit-mr` and
   `cache-miss-expired` but not `cache-miss-timeshift`. The per-event inline enum
   (page 180) has `cache-miss-timeshift` but not `cache-hit-mr` or `cache-miss-expired`.
   `MABR_ObjectDeliveryStatus` (page 183) has `cache-miss-expired` but not
   `cache-hit-mr` or `cache-miss-timeshift`.
2. `MABR_ObjectDeliveryStatus` (page 183) is an **orphan type** — it is defined in the
   schema but is never `$ref`'d by any other component. The per-event
   `object-delivery-status` field (page 180) inlines its own enum literal and does not
   reference this type.
3. `MABR_ObjectDeliveryStatus` contains a **duplicate `cache-miss-expired`** entry
   — it appears in the middle of the list and again at the end — clearly a typo in the
   YAML rather than an intentional extra value.

**Do not pick one silently.** An implementation emitting the clause-11.1.2.2 set will
produce reports containing `cache-hit-mr` / `cache-miss-expired` that a schema-based
receiver rejects; one emitting the Annex N per-event set will produce
`cache-miss-timeshift` that the prose doesn't describe. Until an erratum or V1.3.1
resolves this, an implementation should:
- Accept **all three** sets on the receive side (be lenient).
- Document which set it emits and why.

### ⚠️ Spec quirk: `MABR_Report.required` says `event` (singular)

The YAML on page 179 declares:

```yaml
MABR_Report:
  required:
    - timestamp
    - gateway-id
    - event
```

— **`event`**, not `events`. The actual data property is named `events` (plural, an
array). This is a spec bug (the YAML `required` list names a non-existent property)
that would silently break a strict YAML-to-code generator. An implementation should
treat the `required` list as naming `events` instead.

## 1. HTTP endpoint (clause 11.2)

```
POST /dvb/mabr/reportingInformationInstance HTTP/1.1
Host: <reporting host>
Content-Type: application/json
```

The reporting destination is configured via `MulticastGatewaySessionReporting` /
`ReportingLocator` (see [`mabr-signalling.md`](mabr-signalling.md) §6).

## 2. Document structure (Annex N OpenAPI schema)

### 2.1 Top-level: `MABR_Report`

The root reporting information instance document.

| Property | Required | Type | Description |
|---|---|---|---|
| `timestamp` | yes | string (MPEG-7 TimePoint) | UTC time this report was generated. |
| `gateway-id` | yes | string | Opaque Multicast gateway instance identifier. |
| `gateway-description` | no | string | Human-readable gateway description. |
| `events` | no | array of `MABR_Event` | A list of events to be reported. |
| `playback-sessions` | no | array of `MABR_Session` | Metrics for a set of playback sessions. |

### 2.2 `MABR_Event`

| Property | Required | Type | Description |
|---|---|---|---|
| `timestamp` | yes | string (MPEG-7 TimePoint) | UTC time this event was generated. |
| `playback-session-id` | yes | string | Unique playback session identifier. |
| `type` | yes | enum (see below) | Event type. |
| `object-delivery-status` | yes if type=`object-delivery` | enum (see §2.2.1) | Delivery status of a media object. |
| `service-component-identifier` | yes if type=`service-component-switch` | `MABR_ComponentIdentifier` | Identifies the switched-to service component. |
| `multicast-endpoint-address` | yes if type=`multicast-join` or `multicast-leave` | `MABR_MulticastAddress` | The multicast group joined/left. |

**`type` enum values** (from both clause 11.1.2.2 prose and Annex N — consistent):

| Value | Description |
|---|---|
| `session-started` | Playback session started. No additional objects. |
| `session-ended` | Playback session ended. Contains `session-end-metrics`; may contain `in-session-metrics`. |
| `heartbeat` | Periodic, when no other event to report. |
| `service-component-switch` | Playback switched service component. |
| `multicast-join` | Gateway joined a multicast group. |
| `multicast-leave` | Gateway left a multicast group. |
| `object-delivery` | Media object delivery status report. |

#### 2.2.1 `object-delivery-status` enum (⚠️ CONFLICT — see §flag above)

The Annex N values (9, with `cache-miss-timeshift`):

| Value | Meaning |
|---|---|
| `cache-hit-m` | Object completely received intact via M. |
| `cache-hit-a` | Object already present in cache, previously fetched via A. |
| `cache-miss-incomplete` | Partly received via M, could not be repaired. |
| `cache-miss-filter` | Fetched at A during inactive multicast session. |
| `cache-miss-nodata-s` | No data for this object received via M. |
| `cache-miss-nodata-m` | No session data received via M for this playback session. |
| `cache-miss-nodata-j` | Fetched before gateway subscribed to session. |
| `cache-miss-nodata-o` | Any other unspecified reason. |
| `cache-miss-timeshift` | Object not available via M due to timeshift/catch-up. |

The clause 11.1.2.2 table adds `cache-hit-mr` and `cache-miss-expired` and removes
`cache-miss-timeshift`.

### 2.3 `MABR_Session`

A playback session currently served by the gateway.

| Property | Required | Type | Description |
|---|---|---|---|
| `playback-session-id` | yes | string | Unique playback session identifier. |
| `service-id` | yes | string | `MulticastSession/@serviceIdentifier`. |
| `manifest-id` | yes | string | `PresentationManifestLocator/@manifestId`. |
| `manifest-url` | yes | string | Presentation manifest URL requested at L. |
| `user-agent` | no | string | User agent string. |
| `in-session-metrics` | no | `MABR_InSessionMetrics` | Accumulated metrics since session start. |
| `session-end-metrics` | no | `MABR_SessionEndMetrics` | Final metrics at session end. |

### 2.4 `MABR_InSessionMetrics`

Accumulated metrics collected during a playback session.

| Property | Required | Type | Description |
|---|---|---|---|
| `errors-l` | no | integer ≥0 | Errors at L so far. |
| `errors-a` | no | integer ≥0 | Errors at A so far. |
| `errors-m` | no | integer ≥0 | Errors at M so far. |
| `errors-u` | no | integer ≥0 | Errors at U so far. |
| `cache-rep-a` | yes | integer ≥0 | KBytes of unicast repair data received at A. |
| `cache-rep-f` | yes | integer ≥0 | KBytes recovered via AL-FEC. |
| `cache-rep-u` | no | integer ≥0 | KBytes of unicast repair data received at U. |
| `rep-switch-nb` | yes | integer ≥0 | Number of representation changes. |
| `segment-req-nb` | no | integer ≥0 | Number of segment requests at L. |
| `bytes-received-m` | yes | integer ≥0 | Bytes received at M. |
| `bytes-received-a` | yes | integer ≥0 | Bytes received at A. |
| `bytes-received-l` | no | integer ≥0 | Bytes received at L. |
| `bytes-sent-l` | no | integer ≥0 | Bytes delivered at L. |

### 2.5 `MABR_SessionEndMetrics`

Final metrics at the end of a playback session.

| Property | Required | Type | Description |
|---|---|---|---|
| `service-component-information` | yes | array of `MABR_ServiceComponentInformation` | Per-service-component statistics. |

### 2.6 `MABR_ServiceComponentInformation`

Final metrics about a service component.

| Property | Required | Type | Description |
|---|---|---|---|
| `service-component-identifier` | yes | `MABR_ComponentIdentifier` | Identifies the service component. |
| `multicast-endpoint-address` | no | array of `MABR_MulticastAddress` | Multicast endpoint. |
| `segment-duration` | no | integer ≥1 | Segment duration in ms as declared in manifest. |
| `total-bytes` | yes | integer ≥0 | Bytes delivered at L for this component. |
| `bit-rate` | yes | integer ≥1 | Bit rate from manifest, bits per second. |
| `cache-miss-expired` | yes | integer ≥0 | Objects fetched at A because expired from Asset storage. |
| `cache-miss-incomplete` | yes | integer ≥0 | Objects repaired via A/U because partly received at M. |
| `cache-miss-filter` | yes | integer ≥0 | Objects fetched at A during inactive session. |
| `cache-miss-nodata-m` | yes | integer ≥0 | Objects fetched at A — no session data via M. |
| `cache-miss-nodata-s` | yes | integer ≥0 | Objects fetched at A — not received via M. |
| `cache-miss-nodata-j` | yes | integer ≥0 | Objects fetched at A before subscription. |
| `cache-miss-nodata-o` | yes | integer ≥0 | Objects fetched at A — other reason. |
| `cache-miss-nodata-l` | no | integer ≥0 | Requests at L that could not be fulfilled. |
| `cache-hit-m` | yes | integer ≥0 | Objects from cache received via M. |
| `cache-hit-a` | yes | integer ≥0 | Objects from cache received via A. |
| `cache-hit-mr` | yes | integer ≥0 | Objects from cache received via M, repaired via A/U. |

Note: The per-service-component metrics in `MABR_ServiceComponentInformation` use the
**full set** from clause 11.1.2.5 (which includes `cache-hit-mr` and
`cache-miss-expired`) and are **not** the same enumeration as the per-event
`object-delivery-status` field — `MABR_ServiceComponentInformation` is a set of
numeric counters, not a single enum pick.

### 2.7 `MABR_ComponentIdentifier`

Used to unambiguously identify a service component. One of:

- **`DashComponentIdentifier`**: `period-id` (string, required), `adaptation-set-id`
  (string, required), `representation-id` (string, required).
- **`HlsComponentIdentifier`**: `media-playlist-locator` (string, required).

### 2.8 `MABR_MulticastAddress`

| Property | Required | Type | Description |
|---|---|---|---|
| `source` | no | string | Source address (SSM only). |
| `group` | yes | string | Multicast group IP. |
| `port` | yes | integer 0..65535 | UDP destination port. |
| `transport-session-id` | no | string | Multicast transport session identifier, if configured. |
