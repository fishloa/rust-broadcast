---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 3.3 Client requirements, 3.4 Transport, 3.5 Synchronization, 3.6, 3.7, 3.8 Switching across Adaptation Sets
pdf_pages: 49-54
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch03-3-client-requirements-transport-switching.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 3.3-3.8 client, transport, sync, switching (notes)

PDF p.N = printed p.N-21. Verbatim: private file above.

## 3.3 Client guidelines and requirements (pp.49-51)

- 3.3.1: the IOP is a permission for clients that implement only the required features; how a client conforms is not specified; 3.3.4 gives the minimum.
- 3.3.2 the client's job is: choose Adaptation Sets from descriptors/attributes, make an initial Representation choice per set, fetch (Sub)Segments on time, synchronise components across sets, switch Representations inside a set. Figure 2 contrasts a DASH-AVC/264 client with a full Live+On-Demand profile client: supported = HTTP GET and partial GET, number/time templates and byte-range addressing, MPD and Segment Index metadata, media/init/index download, ISO BMFF parsing, A/V sync, switching at closed-GOP boundaries; not supported = playlist-based addressing, muxed media download, open-GoP switching (figure described, not copied).
- 3.3.3: clients should use the common timeline to present one Representation up to time t and continue another from t; because that can need parallel download/decode, the formats give switching points at (sub)segment boundaries.
- 3.3.4 minimum requirements (shall): switch seamlessly at (sub)segment boundaries as defined in 23009-1 4.5.1 if switching at all; ignore the parent element if the scheme/value of a `FramePacking`, `Rating`, "EssentialDescriptor" (sic; elsewhere `EssentialProperty`) or `ContentProtection` descriptor is not recognised and no equivalent descriptor exists.

## 3.4 Transport (pp.51-52)

- 3.4.1: servers and clients shall support the normative HTTP/1.1 parts, RFC 7230-7234 [21]-[25] (replacing RFC 2616); https/TLS per RFC 5246, see 7.2.
- 3.4.2 servers: should answer byte-range requests; shall do so for Representations of the On-Demand profile; may support the 23009-1 Annex E.3 second-example syntax `BaseURL@byteRange="$base$?$query$&range=$first$-$last$"`.
- 3.4.3 clients: shall issue partial GETs (RFC 7233), may use the Annex E syntax, shall react to HTTP status/error codes as in 23009-1 A.7, should support first-party cookies (RFC 6265).
- 3.4.4 middleboxes: transcoding proxies may break `sidx`/MPD byte ranges; mitigations: encrypted delivery (TLS, segment encryption, content protection) or `Cache-Control: no-transform`; also HTTPS protects the MPD from manipulation. Consider caching impact.

## 3.5 Synchronization (pp.52-53)

Servers and clients should sync to a global time standard so that Segment Availability Times computed per 23009-1 5.3.9.5 and 23009-3 6.4 are accurate. Client time sources: NTP/SNTP (RFC 5905) or the HTTP `Date` header (RFC 7231 7.1.1.2). Allow for source inaccuracy when requesting near availability boundaries. Details: 4.7.

## 3.6-3.7 (p.53)

Pointers only: live services in chapter 4, on-demand in 3.10, ad insertion in chapter 5.

## 3.8 Switching across Adaptation Sets (pp.53-54)

Stated to anticipate 23009-1 Amd.4 clause 5.3.3.5. Sets may offer the same content time-aligned (e.g. H.264 and H.265) so that advanced clients can switch seamlessly across them. Signalling: a SupplementalProperty on the set with scheme `urn:mpeg:dash:adaptation-set-switching:2016` and a comma-separated list of Adaptation Set ids switchable to. If segment/subsegment alignment is true for one set, alignment shall hold for all Representations of all listed sets. Example: set `264` lists `265` and vice versa. For video and audio sets the 3.2.4 presence rules shall also hold for every listed set; consequently switching may be signalled in one direction only (e.g. one set contains all resolutions of the other, but not the reverse). Typo: "Adaption Set". Example quotes are typographic on both sides.
