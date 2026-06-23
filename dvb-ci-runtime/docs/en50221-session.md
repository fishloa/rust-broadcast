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

## Flow (§7.2.3)

**`open_session_request` is always issued by the module** (the "application"),
never by the host (§7.2.2 ¶1). Two cases (figures 9 & 10):

- **Resource the host handles (figure 9 — the normal CI case):** the module sends
  `open_session_request(resource)`; the host allocates a `session_nb` and replies
  `open_session_response(0x00, resource, nb)` directly (or `0xF0`/`0xF1`/… to
  refuse). This is how **every** session in a single-CAM setup is opened —
  resource_manager, application_information, conditional_access, date_time, mmi.
  The host then drives each per its resource protocol (e.g. it sends
  `application_info_enq` on the app-info session).
- **Resource provided by a *second* module (figure 10):** the host extends the
  requester's session onto the provider module's transport connection with a
  **`create_session(resource, nb)`**; that module replies
  `create_session_response`. **`create_session` is host→module routing *only* for
  this inter-module case** — it is **not** how a host uses a module's own
  resource. A single-connection host (this crate's model) never issues it.
- **Data:** once open, each side sends `session_number(nb)` immediately followed by
  one or more APDUs.
- **Close:** either side sends `close_session_request(nb)`; the peer replies
  `close_session_response(nb)`.

The session layer is **mechanism, not policy**: it allocates/tracks `session_nb`s,
**accepts `open_session_request` for any resource the host has a handler for**
(replying `open_session_response`), and routes `session_number`+APDU to/from the
bound resource. *Which* resources are handled is decided by the resource layer.

> **Note (#340).** Earlier wording here said the host opens module-provided
> resources with `create_session`; that is wrong per §7.2.3 — the module opens
> them itself once the RM `profile_change` gate is passed (see
> `en50221-resources.md`). The host only accepts.
