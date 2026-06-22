# EN 50221 Session Layer (§7.2 / §A.5)

_ETSI EN 50221:1997 §7.2. The session layer runs over the transport layer: each
SPDU is carried in one transport SPDU payload. It multiplexes multiple logical
**sessions** (one per resource in use) over the connection._

## SPDU objects (Table 14, `spdu_tag`)

| Object | `spdu_tag` | Issued by | Purpose |
|---|---|---|---|
| `session_number` | `0x90` | either | precedes the APDU body of every data SPDU (`… session_nb · apdu`) |
| `open_session_request` | `0x91` | application (module) | request a session to a **host-provided** resource |
| `open_session_response` | `0x92` | host | grant/deny — carries `session_status` + `session_nb` |
| `create_session` | `0x93` | host | open a session to a **module-provided** resource |
| `create_session_response` | `0x94` | module | grant/deny |
| `close_session_request` | `0x95` | either | close a session `n` |
| `close_session_response` | `0x96` | either | acknowledge the close |

Wire shape: `spdu_tag · length_field · session_object_value`. `open_session_request`
value = `resource_identifier` (4 bytes). `*_response` value = `session_status` (1)
+ `resource_identifier` (4) + `session_nb` (2). `create_session` = resource (4) +
`session_nb` (2). `close_*` = `session_nb` (2). `session_number` value = `session_nb`
(2), **followed by the APDU body**.

## `session_status` (Table 7)

| Value | Meaning |
|---|---|
| `0x00` | session opened |
| `0xF0` | resource non-existent |
| `0xF1` | resource exists but unavailable |
| `0xF2` | resource exists, version lower than requested |
| `0xF3` | resource busy |

## Flow (§7.2)

- **Module → host (open):** the module sends `open_session_request(resource)`; the
  host, if it provides that resource, allocates a `session_nb` and replies
  `open_session_response(0x00, resource, nb)`; otherwise `0xF0`/`0xF1`/…
- **Host → module (create):** to use a module-provided resource the host sends
  `create_session(resource, nb)`; the module replies `create_session_response`.
- **Data:** once open, each side sends `session_number(nb)` immediately followed by
  one or more APDUs.
- **Close:** either side sends `close_session_request(nb)`; the peer replies
  `close_session_response(nb)`. A session may be extended over a second transport
  connection (the `create_session` mechanism); this crate models a single
  connection.

The session layer is **mechanism, not policy**: it allocates/tracks `session_nb`s,
answers `open_session_request` for resources the host advertises, opens
`create_session` on demand, and routes `session_number`+APDU to/from the resource
bound to that session. *Which* resources the host provides vs. requests is decided
by the resource layer.
