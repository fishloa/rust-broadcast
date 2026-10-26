<a id="rfc8470"></a>

# RFC 8470 - Using Early Data in HTTP (the 425 status code), notes for the DASH client

Source: M. Thomson, M. Nottingham, W. Tarreau, "Using Early Data in HTTP", RFC 8470, IETF (Standards Track), September 2018,
<https://www.rfc-editor.org/rfc/rfc8470>. Fetched as plain text 2026-10-26 and read in full (675 lines, sections 1-8).

Licence: IETF documents are published under the IETF Trust's Legal Provisions (BCP 78, <https://trustee.ietf.org/license-info>),
which allow reproduction with attribution. This file is our own-words notes with short quotations marked as such, kept to
the parts a DASH/HLS client needs. Section numbers are RFC 8470's. Statements about what *our* client does are marked **Policy**
and are not from the RFC.

## Why a DASH client meets 425

A live origin or CDN that serves segments before they are announced as available (DASH-IF `livesim2` answers a segment number a few
ahead of the live edge with `425`, body `earl`; recorded in `docs/dash-runtime/DESIGN.md` section 11 item 8) uses the code in the
sense "not now, ask again". The RFC defines it for a different reason (possible replay of TLS 1.3 0-RTT data), so only the
parts below constrain a client; the rest is server and gateway behaviour.

## 5.2 The 425 (Too Early) status code

- Meaning: "the server is unwilling to risk processing a request that might be replayed" (quoted).
- A user agent that sent the request in TLS early data is expected to retry it on 425; it SHOULD retry automatically and the retry MUST NOT
  itself be sent in early data (sections 4 and 5.2).
- A 425 is **not cacheable by default**; its payload is not a representation of any resource (5.2).
- A server SHOULD NOT emit 425 unless the request arrived in early data or carried `Early-Data: 1`, because only then can it assume the
  client is able to retry (5.2).
- An intermediary MUST forward a 425 if the request it forwarded carried an `Early-Data` header; otherwise an intermediary that
  received the request in early data MAY retry it itself, but only after the TLS handshake with its client has completed (5.2).

## 4 Clients that use early data

- Absent other information a client MAY send safe methods (RFC 7231 4.2.1) in early data and MUST NOT send unsafe or unknown-safety
  methods in early data.
- "Clients that use early data MUST retry requests upon receipt of a 425 (Too Early) status code" (quoted).
- If the server rejects early data at the TLS layer the client starts over as though the connection were new and resends what it sent
  (it may abandon those requests).
- An intermediary MUST NOT use early data forwarding a request unless early data was used on a previous hop, or it knows the request can
  be retried safely (typically by configuration).

## 5.1 The `Early-Data` request header

- Grammar: `Early-Data = "1"`; it has exactly one valid value. Example: `Early-Data: 1`.
- Added by an intermediary that forwards a request before its own TLS handshake with the client completed. It MUST NOT be removed if
  present and MUST NOT appear in `Connection`. Not intended for user agents: sending a request in early data already implies the client
  understands the RFC.
- A request carrying it cannot be made safe by waiting for the handshake; if it cannot be processed safely it MUST be rejected with 425.
- At most one instance; a server treats multiple or invalid instances as one instance with value `1`. Never in responses or trailers.

## 3 and 6 (server and gateway side, not client behaviour)

A server may reject early data at TLS, delay processing until the handshake finishes, or answer 425 per request; a server that accepts
early data MUST process every request in it even when it answers one with 425 (3). Gateways MUST NOT forward early-data requests unless
the origin understands `Early-Data` and will generate 425 (6.1); every server instance has to treat a request consistently (6.2).
Early data adds denial-of-service and out-of-order considerations (6.3, 6.4); IANA registers the header and the code (7).

## What the DASH client does with it

**Policy** (not in the RFC; design `docs/dash-runtime/DESIGN.md` sections 5.4 and 8.4):

- The client never sends TLS early data on a segment fetch, so none of the RFC's client MUSTs apply to it; a 425 reaches it only as a
  server's "too early" answer.
- A segment-fetch 425 is classified with 404 and 410 as "not available at this instant": it is a retry condition, never a counted failure
  toward give-up, and the retry backoff is short (`early_retry_us`, 250 ms doubling to 1 s), not the 2/4/8 s failure table.
- The retry is a fresh request on the same connection; because the client does not use early data, the "retry MUST NOT be sent in early
  data" rule holds trivially.
- A 425 response is not cached (5.2); the client does not store it as the outcome of the URL.
- Test requirement (design 8.3 and 8.4): a mutant that counts 425 toward give-up must be killed by a named test.
