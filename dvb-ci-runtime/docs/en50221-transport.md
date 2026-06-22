# EN 50221 Transport Layer (§7.1 / §A.4)

_ETSI EN 50221:1997 §7.1 (transport protocol) + Annex A.4 (object coding). The
transport layer runs over the link layer (the CA device byte stream) and carries
SPDU payloads up to the session layer._

## Transport objects (the eleven, Annex A.4)

| Object | `tpdu_tag` | Dir | Notes |
|---|---|---|---|
| `Create_T_C` | `0x82` | host→mod | open a transport connection; carries `t_c_id` |
| `C_T_C_Reply` | `0x83` | mod→host | reply to Create_T_C (**+ appended T_SB**) |
| `Delete_T_C` | `0x84` | either | close a connection |
| `D_T_C_Reply` | `0x85` | either | reply to Delete_T_C (+ T_SB) |
| `Request_T_C` | `0x86` | mod→host | module asks the host to open a new connection |
| `New_T_C` | `0x87` | host→mod | host grants Request_T_C (carries the new `t_c_id`), immediately followed by a `Create_T_C` |
| `T_C_Error` | `0x88` | host→mod | 1-byte error code (only "no more connections" in v1) |
| `T_SB` | `0x80` | mod→host | **status byte**, see below |
| `T_RCV` | `0x81` | host→mod | request the data the module has waiting |
| `T_Data_More` | `0xA1` | either | one fragment; **at least one more follows** |
| `T_Data_Last` | `0xA0` | either | last/only fragment |

Each object is `tag · length_field · t_c_id [· data…]` (length_field includes the
`t_c_id`). `T_SB` body is `tag · 0x02 · t_c_id · SB_value`.

## T_SB — the status byte (the timing/flow pivot)

- **`T_SB` is the reply to *every* object the host sends** — either appended after
  another object (e.g. `C_T_C_Reply + T_SB`, `T_Data_Last + T_SB`) or **standalone**
  (the reply to a poll when the module has nothing to send).
- `SB_value` carries one meaningful bit: **DA (Data Available, bit 8 / `0x80`)** —
  set when the module has a message queued for the host.

## Poll / receive flow (§A.4, items 8–10)

- The host **polls regularly** with an **empty `T_Data_Last`** (`0xA0`, no data).
- The module replies with `T_SB`. If **DA = 0**, nothing to do — poll again later.
- If **DA = 1**, the host sends **`T_RCV`** (`0x81`); the module then sends its
  queued **`T_Data_*`** (+ T_SB).
- **Module `T_Data_*` is only ever sent in response to a `T_RCV`**, and **one
  fragment per `T_RCV`**: a `T_Data_More` must be followed by another `T_RCV`
  before the next fragment; reassembly ends at `T_Data_Last`. The reassembled
  bytes are one SPDU passed up.

## Host-side state machine (§7.1.3, Figure 6 / Table 2)

| State | Expected from module | Transitions |
|---|---|---|
| **Idle** | none | host sends `Create_T_C` → **In Creation** |
| **In Creation** | `C_T_C_Reply (+ T_SB)` | reply → **Active**; **Timeout → Idle** (host then never polls/transmits on that t_c_id; a late `C_T_C_Reply` is ignored) |
| **Active** | `T_Data_More`, `T_Data_Last`, `Request_T_C`, `Delete_T_C`, `T_SB` | poll/data exchange; `Delete_T_C` → In Deletion |
| **In Deletion** | `D_T_C_Reply (+ T_SB)` | reply → Idle |

A single connection (`t_c_id = 1`) is the normal case; a module may `Request_T_C`
a second connection (host answers `New_T_C` + `Create_T_C`, or `T_C_Error` if none
free).

## Timing

EN 50221 mandates **regular polling** and the **Timeout arc** (Figure 6) out of
*In Creation*, but **does not fix numeric values** — the poll interval and reply
timeout are implementation-chosen. This crate defaults to a 100 ms poll interval
and a 1 s reply timeout, expressed through the sans-IO `Tick`/timer model so they
are deterministic and testable without a real clock.
