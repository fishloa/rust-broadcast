# TCP-Interleaved Session Fixture

**Provenance:** RFC 2326 §10.12 and §14.2 example exchanges. Session id,
CSeq numbering, and stream URI adapted for a single-stream container file
with TCP interleaving. All messages use CRLF line endings (\r\n) as required
by RTSP/1.0.

This fixture documents a complete client-server exchange:

```
OPTIONS → DESCRIBE (with SDP body) → SETUP (TCP interleaved) → PLAY → TEARDOWN
```

---

## Message 1 — OPTIONS request (C→S)

```
OPTIONS rtsp://video.example.com/stream RTSP/1.0\r\n
CSeq: 1\r\n
User-Agent: ExampleClient/1.0\r\n
\r\n
```

## Message 2 — OPTIONS response (S→C)

```
RTSP/1.0 200 OK\r\n
CSeq: 1\r\n
Public: OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN\r\n
\r\n
```

---

## Message 3 — DESCRIBE request (C→S)

```
DESCRIBE rtsp://video.example.com/stream RTSP/1.0\r\n
CSeq: 2\r\n
User-Agent: ExampleClient/1.0\r\n
Accept: application/sdp\r\n
\r\n
```

## Message 4 — DESCRIBE response with SDP body (S→C)

SDP body length: 163 bytes (CRLF-terminated lines, LF-only within SDP body
per RFC 4566 convention; RTSP Content-Length counts all body octets).

```
RTSP/1.0 200 OK\r\n
CSeq: 2\r\n
Content-Type: application/sdp\r\n
Content-Base: rtsp://video.example.com/stream/\r\n
Content-Length: 163\r\n
\r\n
v=0\r\n
o=- 2890844256 2890842807 IN IP4 192.0.2.1\r\n
s=Example Stream\r\n
t=0 0\r\n
a=control:rtsp://video.example.com/stream\r\n
m=video 0 RTP/AVP 96\r\n
a=rtpmap:96 H264/90000\r\n
a=control:trackID=0\r\n
```

---

## Message 5 — SETUP request (C→S, TCP interleaved)

```
SETUP rtsp://video.example.com/stream/trackID=0 RTSP/1.0\r\n
CSeq: 3\r\n
Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n
\r\n
```

## Message 6 — SETUP response (S→C)

```
RTSP/1.0 200 OK\r\n
CSeq: 3\r\n
Session: 12345678\r\n
Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n
\r\n
```

---

## Message 7 — PLAY request (C→S)

```
PLAY rtsp://video.example.com/stream RTSP/1.0\r\n
CSeq: 4\r\n
Session: 12345678\r\n
Range: npt=0-\r\n
\r\n
```

## Message 8 — PLAY response (S→C)

```
RTSP/1.0 200 OK\r\n
CSeq: 4\r\n
Session: 12345678\r\n
RTP-Info: url=rtsp://video.example.com/stream/trackID=0;seq=10001;rtptime=0\r\n
\r\n
```

After the PLAY response, the server sends RTP data as interleaved `$` frames
on channel 0 and RTCP on channel 1. See `interleaved-framing.md` for the frame
format.

---

## Message 9 — TEARDOWN request (C→S)

```
TEARDOWN rtsp://video.example.com/stream RTSP/1.0\r\n
CSeq: 5\r\n
Session: 12345678\r\n
\r\n
```

## Message 10 — TEARDOWN response (S→C)

```
RTSP/1.0 200 OK\r\n
CSeq: 5\r\n
\r\n
```

---

## Notes

- CSeq increments monotonically from 1.
- `Session: 12345678` is set by the server in the SETUP response and echoed
  by the client in all subsequent requests.
- Transport is `RTP/AVP/TCP;unicast;interleaved=0-1`: channel 0 carries RTP,
  channel 1 carries RTCP (RFC 2326 §10.12).
- The aggregate URL `rtsp://video.example.com/stream` is used for PLAY and
  TEARDOWN; the stream URL `.../trackID=0` is used for SETUP (RFC 2326 §14.3).
