# Input modules (StreamInput / ServiceGateway / BroadcastServiceGateway)

_Source: ETSI TS 101 699 V1.1.1 (1999-11) §6.1, Tables 12–34 (PDF pp. 21–40), render-verified_

Input modules are CI extension modules that deliver broadcast services (and the
underlying Transport Streams) into the host. Two flavours are defined:

- **Type 'A'** (§6.1.2) — a simple, potentially low-cost module delivering broadcast
  services over DVB-C/-S/-T at the **TS level**. It presents the **StreamInput**
  resource: the host scans for, and tunes to, transport streams; service selection
  within a TS is the host's responsibility.
- **Type 'B'** (§6.1.3) — a module with a CPU that analyses the network (DVB SI) and
  presents **service-level** access. It presents the **ServiceGateway** (Generic
  Service Gateway) resource, and optionally a network-specific extension. The
  **BroadcastServiceGateway** variant adds broadcast-event (EIT) extensions on top
  of the generic gateway.

Each input module has a **Module ID** and derives its resource_identifier from it
(§6.1.1.3, "iiiiii" = the module_id):

| Resource | resource_identifier form | Example (module_id=1) |
|----------|--------------------------|-----------------------|
| StreamInput (type 'A') | `0x00801ii1` (`0000 0000 1000 0000 0001 iiii ii00 0001`) | — |
| BroadcastServiceGateway (type 'B') | `0x00811ii1` (`0000 0000 1000 0001 0001 iiii ii00 0001`) | `0x00811041` |

(The ServiceGateway / Generic Service Gateway resource is presented on a
well-known resource ID per Table 87; its calls are inherited by all
network-specific gateway resources including BroadcastServiceGateway.)

A `length_field()` is the standard CI APDU ASN.1-style length encoding. The
`TuningInformationMessage` is an 11-byte (`11 x 8`) delivery-system-dependent blob;
for DVB-C/-S/-T it is the trailing 11 bytes of the corresponding DVB SI delivery
system descriptor (§6.1.2.2, Table 15).

---

## StreamInput resource (resource_identifier 0x00801ii1)

Type 'A' input modules present a **StreamInput** resource (resource_identifier
`0x00801ii1`), supporting a single session. Table 12 summarizes the objects.

### Table 12 — Overview of the streamInput objects (p. 25)

| Call | Direction | Description |
|------|-----------|-------------|
| DeliverySystemInfoReq | h → m | Requests the module to provide information on its delivery system. |
| DeliverySystemInfoAck | m → h | Reply describing the type of delivery system connected e.g. (DVB-S, -C, -T) |
| ScanStartReq | h → m | Instructs the module to start scanning for TS |
| ScanNextReq | h → m | Instructs the module to continue scanning for TS |
| ScanAck | m → h | Reply describing the TS found |
| TuneTSReq | h → m | Instructs the module to tune to a TS |
| TuneTSAck | m → h | Reports the success or otherwise of the tune |

### Table 13 — DeliverySystemInfoReq syntax (p. 26)

apdu_tag `DeliverySystemInfoReqTag` = `0x9F 80 00`, Direction host ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `DeliverySystemInfoReq () {` | | |
| &nbsp;&nbsp;DeliverySystemInfoReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 14 — DeliverySystemInfoAck syntax (p. 26)

apdu_tag `DeliverySystemInfoAckTag` = `0x9F 80 01`, Direction app ---> host.

(Table title in the PDF reads "DeliverySystemInfoReq syntax" — a spec typo; the
body and tag describe DeliverySystemInfoAck.)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `DeliverySystemInfoAck () {` | | |
| &nbsp;&nbsp;DeliverySystemInfoAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;SystemIdentifier | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

#### Table 15 — Delivery system identification (p. 26)

| SystemIdentifier value | Delivery system | Tuning information message format |
|------------------------|-----------------|-----------------------------------|
| 0 | "Abstract" | Module specific |
| 1 | DVB-C | As DVB SI cable delivery system descriptor |
| 2 | DVB-S | As DVB SI satellite delivery system descriptor |
| 3 | DVB-T | As DVB SI terrestrial delivery system descriptor |
| > 3 | Reserved for future use | |

Field notes:
- `SystemIdentifier` — 8-bit identifier of the delivery system(s) connected by the
  module (Table 15). The `TuningInformationMessage` is always 11 bytes; for
  values 1–3 it is the last 11 bytes of the corresponding DVB SI delivery system
  descriptor.

### Table 16 — ScanStartReq syntax (p. 27)

apdu_tag `ScanStartReqTag` = `0x9F 80 02`, Direction host ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ScanStartReq () {` | | |
| &nbsp;&nbsp;ScanStartReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 17 — ScanNextReq syntax (p. 27)

apdu_tag `ScanNextReqTag` = `0x9F 80 03`, Direction host ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ScanNextReq () {` | | |
| &nbsp;&nbsp;ScanNextReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 18 — ScanAck syntax (p. 27)

apdu_tag `ScanAckTag` = `0x9F 80 04`, Direction app ---> host.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ScanAck () {` | | |
| &nbsp;&nbsp;ScanAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;TSState | 8 | uimsbf |
| &nbsp;&nbsp;TuningInformationMessage | 11x8 | bslbf |
| &nbsp;&nbsp;ScanProgress | 8 | uimsbf |
| `}` | | |

Field notes:
- `TSState` — unsigned signal-availability indicator. `0` = no signal found (when
  auto-scanning, `0` also indicates all frequencies searched); `1`–`255` = a
  normalized signal-quality value (bigger is better).
- `TuningInformationMessage` — 11-byte delivery-system-dependent coding to
  re-acquire the TS. Undefined when `TSState == 0`.
- `ScanProgress` — 8-bit unsigned (0–255), approximate proportional indication of
  scan progress; increases as the scan progresses.

### Table 19 — TuneTSReq syntax (p. 28)

apdu_tag `TuneTSReqTag` = `0x9F 80 05`, Direction host ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `TuneTSReq () {` | | |
| &nbsp;&nbsp;TuneTSReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;TuningInformationMessage | 11 x 8 | bslbf |
| `}` | | |

Field notes:
- If the `TuningInformationMessage` is absent (length field indicates zero
  following bytes) the request is for the module to disconnect from the network.
- `TuningInformationMessage` — 11-byte coding to acquire the TS; coding identical
  to that returned by ScanAck.

### Table 20 — TuneTSAck syntax (p. 28)

apdu_tag `TuneTSAckTag` = `0x9F 80 06`, Direction app ---> host.

(Table title in the PDF reads "ScanAck syntax" — a spec typo; the body and tag
describe TuneTSAck.)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `TuneTSAck () {` | | |
| &nbsp;&nbsp;TuneTSAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;TSState | 8 | uimsbf |
| `}` | | |

Field notes:
- `TSState` — identical coding to the ScanAck `TSState`. When TuneTSReq carried no
  TuningInformationMessage ("network disconnect") this field is `0` (no signal).

---

## Generic Service Gateway resource (ServiceGateway)

Type 'B' input modules present a **ServiceGateway** (Generic Service Gateway)
resource for service-level access. These calls are inherited by all
network-specific Service Gateway resources. Table 21 summarizes the objects.

### Table 21 — Overview of Application ↔ Resource service interface calls (p. 32)

| Call | Direction | Description |
|------|-----------|-------------|
| ServiceListReq | A → R | Application requests the resource to provide a list of the services that it can supply. |
| ServiceListAck | R → A | The resource gives the application a list of the IDs of the services that it can provide. |
| ServiceListVersionReq | A → R | The application request the version number of the resource's service list. |
| ServiceListVersionAck | R → A | The resource provides the version number of its service list. |
| ServiceListChanged | R → A | The resource notifies the application that its service list has changed. |
| ServiceDescReq | A → R | The application requests further information on a particular service. |
| ServiceDescAck | R → A | The resource supplies further information on a particular service. |
| GetServiceReq | A → R | The application requests the resource to provide a service. |
| GetServiceAck | R → A | The resource replies regarding the availability of a service. |

### Table 22 — ServiceListReq syntax (p. 32)

apdu_tag `ServiceListReqTag` = `0x9F 80 00`, Direction app ---> resource.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceListReq () {` | | |
| &nbsp;&nbsp;ServiceListReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 23 — ServiceListAck syntax (p. 32)

apdu_tag `ServiceListAckTag` = `0x9F 80 01`, Direction resource ---> app.

(Syntax-block name in the PDF reads "ServiceListReq () {" — a spec typo; the tag
and body describe ServiceListAck.)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceListAck () {` | | |
| &nbsp;&nbsp;ServiceListAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;VersionNumber | 8 | uimsbf |
| &nbsp;&nbsp;NumberOfServices | 16 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < NumberOfServices; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;OriginalNetworkID | 16 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ServiceID | 16 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `VersionNumber` — 8-bit integer, increments each time the service list is updated.
- `NumberOfServices` — 16-bit integer, number of service references (may be 0).
- `OriginalNetworkID` — 16-bit original network ID allocated within ETR 162.
- `ServiceID` — 16-bit field uniquely identifying the service within the original
  network. (A service reference is the {OriginalNetworkID, ServiceID} pair;
  transport_stream_id is not required — Figure 12.)

### Table 24 — ServiceListVersionReq syntax (p. 33)

apdu_tag `ServiceListVersionReqTag` = `0x9F 80 02`, Direction app ---> resource.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceListVersionReq () {` | | |
| &nbsp;&nbsp;ServiceListVersionReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 25 — ServiceListVersionAck syntax (p. 34)

apdu_tag `ServiceListVersionAckTag` = `0x9F 80 03`, Direction resource ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceListVersionAck () {` | | |
| &nbsp;&nbsp;ServiceListVersionAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;VersionNumber | 8 | uimsbf |
| `}` | | |

Field notes:
- `VersionNumber` — 8-bit integer, increments each time the service list is updated.

### Table 26 — ServiceListChanged syntax (p. 34)

apdu_tag `ServiceListChangedTag` = `0x9F 80 04`, Direction resource ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceListChanged () {` | | |
| &nbsp;&nbsp;ServiceListChangedTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;VersionNumber | 8 | uimsbf |
| `}` | | |

Field notes:
- `VersionNumber` — 8-bit integer, increments each time the service list is updated.

### Table 27 — ServiceDescReq syntax (p. 34)

apdu_tag `ServiceDescReqTag` = `0x9F 80 05`, Direction app ---> resource.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceDescReq () {` | | |
| &nbsp;&nbsp;ServiceDescReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;OriginalNetworkID | 16 | bslbf |
| &nbsp;&nbsp;ServiceID | 16 | bslbf |
| `}` | | |

### Table 28 — ServiceDescAck syntax (p. 35)

apdu_tag `ServiceDescAckTag` = `0x9F 80 06`, Direction resource ---> app.

(Table title in the PDF reads "ServiceDescReq syntax" — a spec typo; the tag and
body describe ServiceDescAck.)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ServiceDescAck () {` | | |
| &nbsp;&nbsp;ServiceDescAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;OriginalNetworkID | 16 | bslbf |
| &nbsp;&nbsp;ServiceID | 16 | bslbf |
| &nbsp;&nbsp;reserved_future_use | 6 | bslbf |
| &nbsp;&nbsp;EIT_schedule_flag | 1 | bslbf |
| &nbsp;&nbsp;EIT_present_following_flag | 1 | bslbf |
| &nbsp;&nbsp;running_status | 3 | bslbf |
| &nbsp;&nbsp;free_CA_mode | 1 | bslbf |
| &nbsp;&nbsp;descriptors_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (j = 0; j < descriptors_loop_length; j++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- The payload is modelled on the SDT parameters and SDT descriptor loop of a DVB
  broadcast service. `reserved_future_use`, `EIT_schedule_flag`,
  `EIT_present_following_flag`, `running_status`, `free_CA_mode` and
  `descriptors_loop_length` all have meanings identical to the SDT in ETS 300 468.
- Spec inconsistency (RESOLVED, render-verified p. 35): the per-field prose on
  p. 35 states "running_status — This 6 bit field…", but the Table 28 syntax row
  gives `running_status` as **3 bits**. **3 bits is authoritative** — both
  because it is the SDT value (3 bits in ETS 300 468) and because the byte budget
  proves it: `reserved_future_use(6) + EIT_schedule_flag(1) +
  EIT_present_following_flag(1) + running_status(3) + free_CA_mode(1) +
  descriptors_loop_length(12) = 24 bits = 3 bytes`. A 6-bit running_status would
  total 27 bits and break byte alignment. The prose "6 bit" is a spec typo
  (mis-copied from the `reserved_future_use` 6-bit line above it).
- `descriptor()` — a descriptor defined in the SDT in ETS 300 468, or a private
  descriptor in the scope of a private_data_specifier_descriptor. For a DVB
  broadcast service the payload is the descriptors from its SDT; for a service
  gateway service, service_type `0x0D` designates a "service gateway type".

### Table 29 — GetServiceReq syntax (p. 36)

apdu_tag `GetServiceReqTag` = `0x9F 80 07`, Direction app ---> resource.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `GetServiceReq () {` | | |
| &nbsp;&nbsp;GetServiceReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;OriginalNetworkID | 16 | bslbf |
| &nbsp;&nbsp;ServiceID | 16 | bslbf |
| `}` | | |

Field notes:
- If the service reference is absent (length field indicates zero following bytes)
  the request is for the module to disconnect from the network.

### Table 30 — GetServiceAck syntax (p. 36)

apdu_tag `GetServiceAckTag` = `0x9F 80 08`, Direction resource ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `GetServiceAck () {` | | |
| &nbsp;&nbsp;GetServiceAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;OriginalNetworkID | 16 | bslbf |
| &nbsp;&nbsp;ServiceID | 16 | bslbf |
| &nbsp;&nbsp;Reserved | 5 | bslbf |
| &nbsp;&nbsp;ServiceTerminated | 1 | bslbf |
| &nbsp;&nbsp;ServiceNotAvailable | 1 | bslbf |
| &nbsp;&nbsp;CAServiceFlag | 1 | bslbf |
| &nbsp;&nbsp;ActualService | 16 | bslbf |
| `}` | | |

Field notes:
- `Reserved` — 5 bits reserved for future use, set to `0`.
- `ServiceTerminated` — `1` informs the host the service has finished; service
  navigation reverts to the host. Also returned `1` when GetServiceReq carried no
  service reference ("network disconnect").
- `ServiceNotAvailable` — `1` informs the host the requested service is not
  available; navigation reverts to the host.
- `CAServiceFlag` — `1` informs the host that conditional access restrictions apply
  to the delivered service (host must e.g. use CA_PMT to obtain access).
- `ActualService` — 16-bit actual service id being delivered (maps "logical" to
  "actual" services). Zero indicates no valid TS / the host should not attempt to
  decode a service; non-zero = a valid TS is delivered and the value is the
  service ID (MPEG program number) to decode.

#### Table 31 — Allowed combination (p. 37)

| Attribute | Allowed combinations | | |
|-----------|----|----|----|
| ServiceTerminated | 1 | 0 | 0 |
| ServiceNotAvailable | 0 | 1 | 0 |
| CAServiceFlag | 0 | 0 | x |
| ActualService | 0 | 0 | > 0 |

---

## Broadcast Service Gateway resource (resource_identifier 0x00811ii1)

A type 'B' module connected to a broadcast network can present the **Broadcast
Service Gateway** resource (class 129, type 1*, version 1 — resource_identifier
`0x00811ii1`). It **inherits all Generic Service Gateway calls** (Tables 22–31,
tags `0x9F8000`–`0x9F8008`) and adds the broadcast-event (EIT) extension objects
below (§6.1.3.3 Event Presentation). The EIT objects carry the master-registry
tags `0x9F8010` / `0x9F8011`.

### Table 32 — EITSectionReq syntax (p. 39)

apdu_tag `EITSectionReqTag` = `0x9F 80 10`, Direction app ---> module.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `EITSectionReq () {` | | |
| &nbsp;&nbsp;EITSectionReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;TableID | 16 | uimsbf |
| &nbsp;&nbsp;ServiceID | 16 | uimsbf |
| &nbsp;&nbsp;SectionNumber | 8 | uimsbf |
| &nbsp;&nbsp;OriginalNetworkID | 16 | uimsbf |
| &nbsp;&nbsp;Reserved | 7 | bslbf |
| &nbsp;&nbsp;OKToDisruptService | 1 | bslbf |
| `}` | | |

Field notes:
- `TableID` — 16-bit integer (note: wider than the 8-bit EIT table_id; the syntax
  field is 16 bits). Allowed values/definitions are those of the EIT in
  ETS 300 468, i.e. `0x4E`–`0x6F`.
- `ServiceID`, `SectionNumber`, `OriginalNetworkID` — identical meaning to the EIT
  in ETS 300 468 (16/8/16 bits).
- `Reserved` — 7 bits, set to `0`.
- `OKToDisruptService` — `1` means it is acceptable to disrupt delivery of a current
  service to obtain the requested event information; `0` means delivery shall not
  be disrupted.

### Table 33 — EITSectionAck syntax (p. 40)

apdu_tag `EITSectionAckTag` = `0x9F 80 11`, Direction module ---> app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `EITSectionAck () {` | | |
| &nbsp;&nbsp;EITSectionAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;Reserved | 2 | bslbf |
| &nbsp;&nbsp;ResponseCode | 2 | bslbf |
| &nbsp;&nbsp;Length | 12 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < Length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;start_time | 40 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;duration | 24 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;running_status | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;free_CA_mode | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptors_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j = 0; j < descriptors_loop_length; j++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;descriptor() | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

#### Table 34 — EIT Section response code (p. 40)

| value | meaning |
|-------|---------|
| 00 | Section not on the present document (but might be available on another TS) |
| 01 | Section not available |
| 10 | Section found |
| 11 | reserved |

Field notes:
- `Reserved` — 2 reserved bits, set to `0`.
- `ResponseCode` — 2-bit response status (Table 34).
- `Length` — 12-bit integer, number of bytes following it (may be zero, see ETR 211).
- `event_id`, `start_time`, `duration`, `running_status`, `free_CA_mode`,
  `descriptors_loop_length`, `descriptor()` — identical meaning to the EIT defined
  in ETS 300 468 (16 / 40 / 24 / 3 / 1 / 12 bits respectively).

---

## APDU tag summary

| Resource | Object | apdu_tag | Direction | Table |
|----------|--------|----------|-----------|-------|
| StreamInput | DeliverySystemInfoReq | `0x9F8000` | h → m | 13 |
| StreamInput | DeliverySystemInfoAck | `0x9F8001` | m → h | 14 |
| StreamInput | ScanStartReq | `0x9F8002` | h → m | 16 |
| StreamInput | ScanNextReq | `0x9F8003` | h → m | 17 |
| StreamInput | ScanAck | `0x9F8004` | m → h | 18 |
| StreamInput | TuneTSReq | `0x9F8005` | h → m | 19 |
| StreamInput | TuneTSAck | `0x9F8006` | m → h | 20 |
| ServiceGateway | ServiceListReq | `0x9F8000` | A → R | 22 |
| ServiceGateway | ServiceListAck | `0x9F8001` | R → A | 23 |
| ServiceGateway | ServiceListVersionReq | `0x9F8002` | A → R | 24 |
| ServiceGateway | ServiceListVersionAck | `0x9F8003` | R → A | 25 |
| ServiceGateway | ServiceListChanged | `0x9F8004` | R → A | 26 |
| ServiceGateway | ServiceDescReq | `0x9F8005` | A → R | 27 |
| ServiceGateway | ServiceDescAck | `0x9F8006` | R → A | 28 |
| ServiceGateway | GetServiceReq | `0x9F8007` | A → R | 29 |
| ServiceGateway | GetServiceAck | `0x9F8008` | R → A | 30 |
| BroadcastServiceGateway | EITSectionReq | `0x9F8010` | A → m | 32 |
| BroadcastServiceGateway | EITSectionAck | `0x9F8011` | m → A | 33 |

Note: StreamInput and the Generic Service Gateway both reuse the `0x9F8000`-based
tag block; they are disambiguated by the resource (class 128 StreamInput vs the
ServiceGateway / class 129 BroadcastServiceGateway) the session is opened against.
