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
