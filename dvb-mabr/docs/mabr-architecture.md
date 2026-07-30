# DVB-MABR reference architecture — ETSI TS 103 769 V1.2.1 clauses 4-7

Source: ETSI TS 103 769 V1.2.1 (2024-11), "Digital Video Broadcasting (DVB); Adaptive
media streaming over IP multicast". See [`README.md`](README.md) for provenance and
edition history. All clause numbers below are from this edition unless noted.

This document explains **what is transported, by what, to whom** — the logical
functions, the named reference points connecting them, and the deployment/operational
models the rest of the spec (transport annexes, signalling data model) is built on. An
implementer should read this first: the crate's types are expected to correspond to the
functions and messages named here.

## 1. What DVB-MABR is (clause 4, informative)

DVB-MABR ("Multicast ABR") lets a network operator take an existing unicast adaptive
bitrate (ABR) presentation — MPEG-DASH or HLS — and deliver its media objects (segments,
initialization segments, presentation manifests) over **IP multicast** to a
**Multicast gateway** close to the viewer, which then serves them back to an unmodified
ABR player over ordinary unicast HTTP. The player is unaware multicast was involved.
Packet loss on the multicast path is recovered by an optional AL-FEC scheme and/or a
unicast repair fallback.

## 2. Logical functions (clause 5.3)

| Function | Role | Spec clause |
|---|---|---|
| Content preparation | Encodes, optionally encrypts, and packages source media into DASH/HLS segments. | 5.3.1 |
| Content hosting | Serves prepared content over unicast HTTP(S); origin/CDN for pull ingest, cache-miss fill, and player fallback. | 5.3.2 |
| Multicast server | Ingests content (push or pull) and encapsulates it into multicast transport objects, transmitted over IP multicast. Source-Specific Multicast (SSM). | 5.3.3 |
| Unicast repair service | Repairs multicast packet loss for a Multicast gateway, either from its own cache of the multicast stream, from the Multicast server, or from Content hosting. | 5.3.4 |
| Multicast gateway | Receives multicast transport objects, reconstructs them (with FEC/unicast repair as needed) into **playback delivery objects**, and serves those to the Content playback function over unicast HTTP. May be a forward proxy or local origin/reverse proxy. | 5.3.5 |
| Provisioning | Collects service reporting, configures Network resources, and configures both the Multicast server and Multicast gateway population. | 5.3.6 |
| Content Provider control | Provisions which services are available over multicast, via the Network control function. | 5.3.7 |
| Content playback | The ABR media player: requests and renders media objects; unicast-only; delivery-path-agnostic. | 5.3.8 |
| Multicast rendezvous service | Handles the player's initial presentation-manifest request; decides whether to redirect it to a Multicast gateway (multicast available) or to Content hosting (unicast only). | 5.3.9 |
| DRM licence management | Optional; supplies encryption keys / licences. Out of scope for wire format. | 5.3.10 |
| Application | Controls the Content playback function (e.g. an EPG); out of scope. | 5.3.11 |
| Service directory | Application-specific service discovery; out of scope. | 5.3.12 |

Functions drawn in the reference architecture diagram (figure 5.2-1) with grey text
(DRM licence management, Application, Service directory, Content Provider control,
Provider metrics reporting capture) are **out of scope** of the present document even
though the diagram shows them for context.

Note: the reference-architecture diagrams in clauses 5.1.0, 5.2, 6.1-6.3 and 7.1-7.2 are
box-and-line figures. The PDF->text extraction used to produce this transcription
(pdf2md, textlayer engine) cannot recover diagram layout — the figure text renders as
scrambled word fragments in the raw conversion and is **not reproduced**; only the prose
clauses (which read as ordinary paragraphs, not table/diagram cells) are transcribed
below. The named reference points and step lists are taken from that prose, not from the
diagrams.

## 3. Reference points (clause 5.1)

Reference points are named single-letter (or letter-prime) labels; each is a concrete
interface using a specific protocol in a real deployment.

### 3.1 Data plane (clause 5.1.1)

| Ref. point | Between | Purpose |
|---|---|---|
| `L` | Content playback -> Multicast gateway | Unicast HTTP(S) fetch of all content types. May be a local API if gateway and playback are co-located (clause 6.3). |
| `B` | Content playback -> Multicast rendezvous service | Bootstrap unicast HTTP(S) request for the presentation manifest at session start (clause 7.5). |
| `A` | Multicast gateway -> Content hosting | Unicast acquisition of content not sent over `M`, and fallback when `U` cannot repair (clause 9). |
| `A'` | Content playback -> Content hosting | Unicast retrieval of content out of scope for `L`. |
| `A''` | Unicast repair service -> Content hosting | Retrieval used by the repair service to effect content repair. |
| `M` | Multicast server -> Multicast gateway (and optionally Unicast repair service) | IP multicast content transmission. |
| `U` | Multicast gateway's Unicast repair client -> Unicast repair service | Unicast repair requests/payloads. |
| `U'` | Unicast repair service -> Multicast server | Alternative repair-payload source to `A`. |
| `Pin` | Content packaging -> Content hosting | Publication of prepared content (push or pull-on-demand). |
| `Oin` | Multicast server -> Content hosting | Ingest of content by the Multicast server (typically pull). |
| `Pin'` | Content packaging -> Multicast server | Direct ingest of content by the Multicast server (typically push). |

### 3.2 Control plane (clause 5.1.2)

| Ref. point | Purpose |
|---|---|
| `CMS` | Configuration of the Multicast server. |
| `CMR` | Configuration of the Multicast gateway. |
| `CCP` | Configuration of the Provisioning function by Content Provider control. |
| `RS` | Service reporting: Multicast gateway -> Service reporting capture. |
| `RCP` | Service reporting: Service reporting capture -> Content Provider metrics reporting capture. |
| `RPM` | Playback metrics reporting: Content playback -> Content Provider metrics reporting capture. |

## 4. Deployment models (clause 6, informative)

Three placements of the Multicast gateway relative to the home network, differing only
in *where* the multicast-to-unicast conversion happens; the reference architecture and
reference points are unchanged:

1. **Network edge device** (clause 6.1) — gateway upstream of the home, serving several
   homes; all traffic between edge device and home gateway is unicast.
2. **Home gateway device** (clause 6.2) — gateway in the ISP-supplied router, serving
   multiple terminal devices in one home.
3. **Terminal device** (clause 6.3) — gateway and Content playback co-located in the
   same end device (e.g. set-top box); the home gateway only performs multicast group
   subscription. In this model the Multicast gateway shall serve only its host device.

## 5. Modes of system operation (clause 7)

### 5.1 Regular vs. co-located deployment (clauses 7.1-7.2)

- **Regular deployment**: the Multicast rendezvous service is a separate,
  network-operated function.
- **Co-located deployment**: the Multicast rendezvous service is integrated into the
  Multicast gateway itself (mandatory for unidirectional/one-way deployments, since
  there is no back channel to a remote rendezvous service).

Both follow the same 15-step workflow (clause 7.1, figure 7.1-1 / clause 7.2, figure
7.2-1): Network control configures the Multicast server (1) -> server ingests and sends
media (2-3) -> gateway becomes discoverable (4) -> Application resolves a presentation
manifest URL, optionally augmented by locally-discovered gateway info (5) -> playback
function requests the manifest via `B` (7-8) -> the rendezvous service 307-redirects to
the gateway (or to Content hosting if the service isn't multicast-enabled) -> gateway
subscribes to the relevant multicast transport session(s) (10) -> gateway serves the
manifest and subsequent segment requests over `L` (12-15), falling back to `A`/`A'` on
cache miss.

### 5.2 Gateway/rendezvous discovery (clauses 7.3-7.4, informative)

- **Local system discovery**: an Application may discover a Multicast gateway (and any
  co-located rendezvous service) via multicast DNS (IETF RFC 6762), then piggyback the
  discovered IP/port/operator-id as query parameters on the presentation-manifest
  request URL (clause 7.5.1).
- **Third-party CDN broker redirect**: the manifest URL may first point at a broker that
  redirects to a suitable Multicast rendezvous service, optionally carrying an
  authentication token.

### 5.3 Rendezvous request/redirect syntax (clause 7.5, normative)

Request URL (clause 7.5.1):

```
http[s]://<Host>/<ManifestPath>[?<field>=<value>[&<field>=<value>]*]
```

Recognised query fields: `AToken` (auth token, optional), `MGstatus` (0=inactive,
1=active, optional), `MGid` (`[IP address]:port` of the gateway, optional), `MGhost`
(gateway hostname, optional), `Ori` (original targeted host, optional). Query
components must comply with IETF RFC 3986 Appendix A production rules.

On success the rendezvous service returns (clause 7.5.2.1):

```
HTTP/1.1 307 Temporary Redirect
Location: http[s]://<Multicast gateway>[/<Session ID>]/<ManifestPath>[?conf=<...>]
```

`conf` carries a **multicast gateway configuration instance document** (clause 10.2,
one `MulticastSession`) Gzip-compressed then base64url-encoded (IETF RFC 4648 §5) — the
"just-in-time" configuration method (clause 10.1.2 method 4). On refusal: `401
Unauthorized` (bad/missing auth token) or `404 Not Found` (manifest not known) (clause
7.5.2.2).

### 5.4 Dynamic gateway registration (clause 7.6, normative)

A gateway performing the out-of-band-pulled configuration method (clause 10.1.2 method
2) may append query parameters to its `CMR` request to self-register with the
Provisioning function: `content-playback-subnet` (0..*, an IPv4/IPv6 subnet string the
gateway is a valid redirect target for — never a private/link-local/loopback range) and
`redirect-base-url` (0..1, base URL the rendezvous service should use when redirecting
players to this gateway). If the gateway stops polling, Provisioning removes it from the
rendezvous configuration.

## 6. Scope note for the implementer

This document (architecture) does not specify any wire bytes. The two things an
implementer actually parses/builds are covered in the companion documents:

- [`mabr-transport.md`](mabr-transport.md) — the multicast transport object formats
  (FLUTE and ROUTE profiles), FEC, and integrity/authenticity metadata (clauses 8, 9,
  12, Annexes F/H).
- [`mabr-signalling.md`](mabr-signalling.md) — the multicast session configuration XML
  document (clause 10, Annexes A/B) that binds the two together (which transport
  protocol, which endpoint addresses, which service components).

See [`README.md`](README.md) for what could not be established from a readable source.
