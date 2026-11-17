# rtmp-runtime 0.11.1

_Released 2026-11-17._

Patch release from the audit 20 bounds bundle. Command Messages are capped at 64 KiB before AMF0 parsing, and AMF0 decoding counts values against a 4096 cap, so an 8 MiB pre-authentication body can no longer expand into millions of values and strings. The two new limits report `RtmpError::CommandTooLong` and `RtmpError::Amf0TooManyValues`.

## Fixed

- Command Messages are capped at `server::MAX_COMMAND_LEN` (64 KiB) before AMF0 parsing, and AMF0 decoding counts every value in the sequence against `amf0::MAX_AMF0_VALUES` (4096), so an 8 MiB pre-authentication body of one-byte nulls or three-byte empty-key pairs can no longer expand into millions of values and strings. Both limits report the new typed `RtmpError::CommandTooLong` / `RtmpError::Amf0TooManyValues` (#2813).

---

Published from tag `rtmp-runtime-v0.11.1`.
