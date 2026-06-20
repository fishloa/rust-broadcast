# Host Control resource (tune / replace / clear_replace / ask_release)

_Source: EN 50221 §8.5.1, Tables 27-30 (PDF pp. 33-34), render-verified_

The DVB Host Control resource (resource identifier `00200041`, see Table 57) gives
an application limited control over the host: re-tuning to a different service, and
temporarily replacing one service component by another from the same multiplex.
Only the host provides the DVB Host Control resource and it can only support one
session at a time (EN 50221 §8.5.1, p. 33).

apdu_tag values (cross-ref Table 58, `apdu-tag-values.md`):

| apdu_tag | tag value | Direction (host <-> app) |
|----------|-----------|--------------------------|
| Ttune          | `9F 84 00` | `<---` |
| Treplace       | `9F 84 01` | `<---` |
| Tclear_replace | `9F 84 02` | `<---` |
| Task_release   | `9F 84 03` | `--->` |

## Table 27 — Tune object coding (tune)

apdu_tag `Ttune` = `9F 84 00`, Direction `<---` (EN 50221 §8.5.1.1, Table 27, PDF p. 33).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune () {` | | |
| &nbsp;&nbsp;tune_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;network_id | 16 | uimsbf |
| &nbsp;&nbsp;original_network_id | 16 | uimsbf |
| &nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| `}` | | |

Allows the application to have the host tune to a different service. The parameters
`network_id`, `original_network_id`, `transport_stream_id` and `service_id` are
defined in reference [4] (ETSI EN 300 468). When the host has tuned to the new
service it must enter into the standard CA support dialogue using the CA_PMT object
(§8.4.4) to enable the new service to be descrambled. Sending this object may cause
the host to lose its current state; if so, the host will not re-tune to the previous
service when the DVB Host Control session is closed.

## Table 28 — Replace object coding (replace)

apdu_tag `Treplace` = `9F 84 01`, Direction `<---` (EN 50221 §8.5.1.2, Table 28, PDF p. 34).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `replace () {` | | |
| &nbsp;&nbsp;replace_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;replacement_ref | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;replaced_PID | 13 | uimsbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;replacement_PID | 13 | uimsbf |
| `}` | | |

Replace (and Clear Replace) allow one service component to be temporarily replaced by
another from the same multiplex.

- `replacement_ref` — a value allocated by the application, used to match a Clear
  Replace object with one or more previous Replace objects. Several Replace objects
  can use the same `replacement_ref`, in which case they are all cleared together
  when the matching Clear Replace object is sent.
- `replaced_PID` — the PID of the component (video, audio, teletext or subtitles) to
  be replaced.
- `replacement_PID` — the PID of the component to replace it with. The replacement
  occurs immediately.

The host retains the context for the previous services so it can reinstate them when
Clear Replace is sent; the previous context is also restored, if possible, when the
session to the DVB Host Control resource is closed.

## Table 29 — Clear Replace object coding (clear_replace)

apdu_tag `Tclear_replace` = `9F 84 02`, Direction `<---` (EN 50221 §8.5.1.3, Table 29, PDF p. 34).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `clear_replace () {` | | |
| &nbsp;&nbsp;clear_replace_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;replacement_ref | 8 | uimsbf |
| `}` | | |

- `replacement_ref` — matches the value used in one or more previous Replace objects;
  all Replace operations sharing this reference are undone.

## Table 30 — Ask Release object coding (ask_release)

apdu_tag `Task_release` = `9F 84 03`, Direction `--->` (EN 50221 §8.5.1.4, Table 30, PDF p. 34).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ask_release () {` | | |
| &nbsp;&nbsp;ask_release_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

Sent by the host to the application when it needs to regain control while the
application has a Host Control session in progress. The application is given a short
time-out period to send Clear Replace objects, if necessary, and close the session.
If the session is not closed at the end of the time-out period the host will close it
anyway and restore the previous host state, if possible. `length_field()` = 0 (no
payload).

## Notes

- This resource carries no separate value-code tables — `replacement_ref` is an
  opaque application-allocated reference and the tune identifiers are defined by
  reference [4].
- §8.5.2 Date and Time (Tables 31-32) is a separate resource and is transcribed in
  `datetime.md`; it is not part of Host Control.
