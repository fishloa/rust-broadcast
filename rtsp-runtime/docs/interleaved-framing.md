# RTSP Embedded (Interleaved) Binary Data

**Source:** RFC 2326 §10.12

Certain firewall designs and other circumstances may force a server to interleave
RTSP methods and stream data on the same TCP connection. This interleaving should
generally be avoided unless necessary, as it complicates client and server
operation and adds overhead. Interleaved binary data **SHOULD only be used if RTSP
is carried over TCP**.

---

## Frame Format

Each interleaved data block is prefixed with a 4-byte header:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    '$' (0x24) |  channel id   |       length (big-endian)     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   data (length bytes) ...                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Byte(s) | Field       | Description                                               |
|---------|-------------|-----------------------------------------------------------|
| 0       | `$` (0x24)  | ASCII dollar sign — magic byte identifying an interleaved block |
| 1       | channel id  | 1-byte channel identifier (as negotiated via `interleaved=` in Transport) |
| 2–3     | length      | 2-byte big-endian integer: number of bytes that follow    |
| 4…      | data        | Exactly `length` bytes of RTP or RTCP payload, including the upper-layer protocol header |

The stream data follows **immediately** after the 4-byte prefix, without a CRLF.
Each `$` block contains exactly **one upper-layer protocol data unit** (e.g. one
RTP packet).

Method names may not start with a `$` character (RFC 2326 §10 intro), so an RTSP
parser can unambiguously distinguish a framed data block from a response or
request line.

---

## Channel Mapping

The channel identifier is defined in the SETUP `Transport` header via the
`interleaved=` parameter (RFC 2326 §12.39):

```
Transport: RTP/AVP/TCP;interleaved=0-1
```

Convention (from §10.12):

- **Even channel** → RTP data packets.
- **Odd channel (= RTP channel + 1)** → RTCP packets.

As a default, RTCP packets are sent on the **first available channel higher than
the RTP channel**. The client MAY explicitly request RTCP on another channel by
specifying a two-channel range in `interleaved=`.

This allows RTP/RTCP to be handled similarly to the UDP case — one channel for
data, one for control — but tunnelled over the RTSP TCP connection.

---

## Example Exchange (RFC 2326 §10.12)

```
C->S: SETUP rtsp://foo.com/bar.file RTSP/1.0
      CSeq: 2
      Transport: RTP/AVP/TCP;interleaved=0-1

S->C: RTSP/1.0 200 OK
      CSeq: 2
      Date: 05 Jun 1997 18:57:18 GMT
      Transport: RTP/AVP/TCP;interleaved=0-1
      Session: 12345678

C->S: PLAY rtsp://foo.com/bar.file RTSP/1.0
      CSeq: 3
      Session: 12345678

S->C: RTSP/1.0 200 OK
      CSeq: 3
      Session: 12345678
      Date: 05 Jun 1997 18:59:15 GMT
      RTP-Info: url=rtsp://foo.com/bar.file;seq=232433;rtptime=972948234

S->C: $\x00{2-byte length}{RTP packet with header}   ; channel 0 = RTP
S->C: $\x00{2-byte length}{RTP packet with header}   ; channel 0 = RTP
S->C: $\x01{2-byte length}{RTCP packet}              ; channel 1 = RTCP
```

---

## Parsing Guidance for the Engine

When reading from a TCP stream carrying RTSP:

1. Peek at the next byte.
   - If it is `$` (0x24): read 3 more bytes to get the channel id and 2-byte
     length; then read exactly `length` bytes as the payload. Dispatch to the
     appropriate RTP or RTCP handler based on channel id.
   - Otherwise: accumulate bytes until `\r\n\r\n`; parse as an RTSP
     request or response.
2. RTSP responses/requests and interleaved data may be freely interleaved on the
   same connection; the `$` magic byte is the framing discriminator.
