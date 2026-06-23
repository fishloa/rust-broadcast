# EN 50221 Resources (§8)

_ETSI EN 50221:1997 §8. Resources are the application-layer services exchanged as
APDUs over sessions. Each resource is identified by a `resource_identifier()`._

## Provider split (who opens the session)

| Resource | `resource_id` | Provider | Session opened by |
|---|---|---|---|
| Resource Manager | `0x00010041` | **host** | module (`open_session_request`) |
| Application Information | `0x00020041` | **module** | host (`create_session`) |
| Conditional Access Support | `0x00030041` | **module** | host (`create_session`) |
| Host Control | `0x00200041` | host | module |
| Date-Time | `0x00240041` | **host** | module |
| MMI | `0x00400041` | **module** | host (`create_session`) |
| Low-Speed Comms | `0x00600041` | either | — |

So the host **advertises** (answers `open_session_request` for) Resource Manager,
Date-Time, Host Control; and **opens** (`create_session`) sessions to the
module-provided Application Information, Conditional Access, and MMI once it learns
the module provides them.

## Resource Manager protocol (§8.4.1.1)

The RM is host-provided and has no session limit (always granted). The exact
sequence, transcribed from EN 50221 §8.4.1.1 (the **`profile_change` gate** is the
crux — without it the module never proceeds):

1. The module (application/resource-provider) requests a session to the Resource
   Manager. It is always granted (no session limit).
2. The RM (host) sends **`profile_enq`** (`9F 80 10`); the module replies
   **`profile`** (`9F 80 11`) listing the resources **it** provides.
3. **The module must now wait for a `profile_change` object. While waiting it can
   neither create sessions to other resources nor accept sessions** — it answers
   `resource non-existent` / `resource exists but unavailable`. *(verbatim sense
   of §8.4.1.1 ¶1)*
4. When the host has all profile replies it builds its resource list and **sends a
   `profile_change` (`9F 80 12`) on all active RM sessions.**
5. On receiving `profile_change` for the first time, the module **may interrogate
   the host** (`profile_enq` → host replies `profile` with the host's list) and is
   **now free to create or accept other sessions** — i.e. it opens sessions to
   `application_information`, `conditional_access`, `mmi`, etc.
6. The RM session persists for later `profile_change` notifications.

> **Implementation note (#337/#340).** The host MUST send `profile_change` after
> the module's `profile` reply — that is what unblocks the module to open its
> resource sessions. A real AlphaCrypt/Irdeto CAM sends its `profile` then idles
> until it gets `profile_change`; it does **not** enquire the host first, and it
> does **not** wait for the host to open anything (per §7.2.3 the module opens its
> own sessions; the host's `create_session` is only for routing to a *second*
> module — see `en50221-session.md`).

## Application Information (§8.4.2)

`application_info_enq` (`9F 80 20`) → `application_info` (`9F 80 21`):
`application_type` (0x01 = CA), `application_manufacturer` (2), `manufacturer_code`
(2), and a text `menu_string`.

## Conditional Access Support (§8.4.3)

`ca_info_enq` (`9F 80 30`) → `ca_info` (`9F 80 31`): the list of `CA_system_id`s the
module can descramble. The host sends `ca_pmt` (`9F 80 32`, built by
`dvb_ci::build_ca_pmt`); the module may return `ca_pmt_reply` (`9F 80 33`).

## Date-Time (§8.4.4)

The module sends `date_time_enq` (`9F 84 40`) with a `response_interval`; the host
replies `date_time` (`9F 84 41`, UTC MJD+BCD) immediately and then every
`response_interval` seconds (interval 0 = on request only). The host owns this
timer — it is driven by the sans-IO `Tick` model.

## MMI (§8.6)

`mmi` objects (`9F 88 xx`): `menu`/`list`/`enq` from the module to display, and
`menu_answ`/`answ`/`close` from the host. Surfaced to the application via
`Notification::Mmi`.
