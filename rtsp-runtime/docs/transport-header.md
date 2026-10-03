# RTSP Transport Header

**Source:** RFC 2326 §12.39

The `Transport` request header indicates which transport protocol is to be used
and configures its parameters: destination address, multicast TTL, destination
port, interleaving channel, etc. It sets values not already determined by a
presentation description.

Transports are comma-separated, listed in order of preference. Parameters within
each transport are semicolon-separated.

The Transport header MAY also be used to change certain transport parameters of an
existing stream; a server MAY refuse to do so.

The server MAY return a `Transport` response header indicating the values actually
chosen. A Transport request header field may contain a list of transport options
acceptable to the client; in that case the server MUST return the single option
actually chosen.

---

## ABNF Grammar

(RFC 2326 §12.39, lines ~3380–3404)

```
Transport           =    "Transport" ":"
                         1#transport-spec
transport-spec      =    transport-protocol/profile[/lower-transport]
                         *parameter
transport-protocol  =    "RTP"
profile             =    "AVP"
lower-transport     =    "TCP" | "UDP"
parameter           =    ( "unicast" | "multicast" )
                    |    ";" "destination" [ "=" address ]
                    |    ";" "interleaved" "=" channel [ "-" channel ]
                    |    ";" "append"
                    |    ";" "ttl" "=" ttl
                    |    ";" "layers" "=" 1*DIGIT
                    |    ";" "port" "=" port [ "-" port ]
                    |    ";" "client_port" "=" port [ "-" port ]
                    |    ";" "server_port" "=" port [ "-" port ]
                    |    ";" "ssrc" "=" ssrc
                    |    ";" "mode" = <"> 1#mode <">
ttl                 =    1*3(DIGIT)
port                =    1*5(DIGIT)
ssrc                =    8*8(HEX)
channel             =    1*3(DIGIT)
address             =    host
mode                =    <"> *Method <"> | Method
```

The transport-spec syntax is:

```
transport/profile/lower-transport
```

For `RTP/AVP`, the default lower-transport is **UDP**.

---

## Parameters

### General parameters

**`unicast` | `multicast`** (mutually exclusive)
Indicates whether unicast or multicast delivery will be attempted. Default is
`multicast`. Clients capable of both MUST include two full transport-specs with
separate parameters for each.

**`destination`**
The address to which a stream will be sent. The client may specify a multicast
address. A server SHOULD authenticate the client and log attempts before allowing
a client-specified destination different from the command source address.

**`source`**
If the source address for the stream differs from the RTSP endpoint address
(server for playback, client for recording), it MAY be specified here.

**`interleaved=channel[-channel]`**
Mixes the media stream with the RTSP control stream over the same TCP connection
(see §10.12 / `interleaved-framing.md`). The argument gives the channel number
used in the `$` framing. Specified as a range (e.g. `interleaved=0-1`) so that
both RTP (even channel) and RTCP (odd channel) can be carried.

**`mode`**
The methods to be supported for this session. Valid values: `PLAY` and `RECORD`.
Default is `PLAY` if not provided.

**`append`**
When `mode` includes `RECORD`: media data should be appended to the existing
resource rather than overwriting it. If appending is requested but not supported,
the server MUST refuse rather than overwrite.

### Multicast-specific

**`ttl=N`**
Multicast time-to-live (1–255).

**`layers=N`**
Number of multicast layers to use. Layers are sent to consecutive addresses
starting at `destination`.

**`port=lo-hi`**
The RTP/RTCP port pair for a multicast session (e.g. `port=3456-3457`).

### RTP-specific (unicast)

**`client_port=lo-hi`**
The unicast RTP/RTCP port pair on which the client has chosen to receive media
data and control information (e.g. `client_port=3056-3057`).

**`server_port=lo-hi`**
The unicast RTP/RTCP port pair on which the server has chosen to send/receive
media data and control information (e.g. `server_port=5000-5001`).

**`ssrc=HHHHHHHH`**
The RTP SSRC value (8 hex digits) that should be (request) or will be (response)
used by the media server. Only valid for unicast transmission.

---

## Example header lines from the RFC

UDP unicast with client/server port negotiation (§14.1):

```
Transport: RTP/AVP/UDP;unicast;client_port=3056-3057
Transport: RTP/AVP/UDP;unicast;client_port=3056-3057;server_port=5000-5001
```

TCP interleaved with RTP on channel 0, RTCP on channel 1 (§10.12):

```
Transport: RTP/AVP/TCP;interleaved=0-1
```

Multicast with destination, ports, TTL, and mode (§12.39 example):

```
Transport: RTP/AVP;multicast;ttl=127;mode="PLAY",
           RTP/AVP;unicast;client_port=3456-3457;mode="PLAY"
```

Multicast recording (§14.6):

```
Transport: RTP/AVP;multicast;destination=224.0.1.11;port=21010-21011;mode=record;ttl=127
```

---

## Parser decisions (owner decision (c), 2026-10-03)

rtsp-runtime owns the parser and canonical serializer for this header
(`src/transport.rs`, lexer `src/rfc2326_lex.rs`, RFC 2326 §15.1; full text in
`rfc2326.md`). Input is lenient, output canonical:

- transport / profile / lower-transport tokens and parameter names are case-insensitive
  (`rtp/avp/tcp`, `Interleaved=0-1`, `UNICAST`, `mode=record` as in RFC §14.6);
- implied LWS around `/`, `;`, `,`, `=` and `-`; commas inside quoted-strings never split specs;
- any value may be quoted (`ssrc="DEADBEEF"`); `mode` is accepted unquoted as well as
  quoted although the grammar requires `<">`; the list is comma-separated;
- `interleaved=6`, `client_port=10000` (one end) mean `lo = hi`; a bare `destination` is kept;
- range errors: port > 65535 (max 5 digits), ttl > 255 (3 digits), channel > 255 (3 digits),
  `ssrc` not exactly 8 hex digits, a range with more than two ends, a value-less
  `interleaved`/`ttl`/`port`/…, a value on `unicast`/`multicast`/`append`;
- only `RTP/AVP/{TCP|UDP}` is modelled; other protocols/profiles are errors;
- unknown parameters are preserved in order; a repeated known parameter: last wins.

Canonical output order: transport-spec, `unicast|multicast`, `destination`, `source`,
`interleaved`, `append`, `ttl`, `layers`, `port`, `client_port`, `server_port`, `ssrc`
(8 upper-case hex), `mode="PLAY,RECORD"`, then the unknown parameters. Invariants (tested and
fuzzed): parse -> serialize -> parse equal; serialize -> parse equal; canonical form idempotent.

## rtsp-types gaps (rtsp-types 0.1.3; material for an upstream issue)

Probed with `typed_header::<Transports>()` / `typed_header::<Session>()` (raw output:
`.delegate/rtsp-types-probe.txt`):

| input | rtsp-types result |
|---|---|
| `Session: abc; timeout=30` | id `abc`, timeout **None** (no LWS trimming) |
| `Session: abc ; timeout=30` | id `"abc "` (trailing space), timeout None |
| `Session: abc;timeout = 30`, `abc;TIMEOUT=30` | timeout None |
| `Session: abc;timeout=oops` | whole header **rejected** (session id lost) |
| `Transport: rtp/avp/tcp;interleaved=0-1` | `Other { spec: "rtpavptcp" }` |
| `...;Interleaved=0-1`, `;UNICAST`, `;SSRC=DEADBEEF` | parsed, but the parameter lands in the unknown map (interleaved None, unicast false, ssrc empty) |
| `...;ssrc="DEADBEEF"` | rejected |
| duplicate parameter | `FIXME: we assume each parameter appears only once` |
| `layers` | `TODO layers` (only via the unknown map) |
| unterminated header block (70 KiB of `A`) | `Message::parse` returns `Incomplete(Some(1))`, so callers cannot cap header size from the Some/None distinction |
