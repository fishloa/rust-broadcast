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

The RM is host-provided and has no session limit (always granted). On a Resource
Manager session:

1. The RM (host) sends **`profile_enq`** (`9F 80 10`) to the module.
2. The module replies **`profile`** (`9F 80 11`) — the list of resources **it**
   provides.
3. The module sends the host a **`profile_enq`**; the host replies **`profile`**
   with the host's resource list.
4. Once profiles are exchanged the handshake is complete (**CamReady**); the host
   then opens sessions to the module-provided resources it understands.
5. **`profile_change`** (`9F 80 12`) on an active RM session signals a resource set
   changed → re-enquire.

> **Implementation note (#337).** In practice many real CAMs (e.g.
> AlphaCrypt/Irdeto) send only their `profile` reply (step 2) and then idle — they
> never perform step 3 (enquiring the host's profile). The runtime therefore
> treats the handshake as complete on the **module's profile alone** (step 2),
> and still answers a host `profile_enq` independently if one ever arrives.

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
