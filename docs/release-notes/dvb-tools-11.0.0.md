# dvb-tools 11.0.0

_Released 2026-10-05._

**Major by lockstep; two output fixes, no flag or format change.** `dvb-tools` is an application, so the major bump only reflects the lockstep (its dependencies move to `dvb-si` 11, `dvb-t2mi` 11, `mpeg-ts` 0.5). Two subcommands now give correct results on multi-program captures. Read with [dvb-si 11.0.0](dvb-si-11.0.0.md), [dvb-t2mi 11.0.0](dvb-t2mi-11.0.0.md) and [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md).

```toml
-dvb-si   = { version = "10",  features = ["ts", "serde", "chrono"] }
-dvb-t2mi = { version = "10",  features = ["ts"] }
-mpeg-ts  = { version = "0.4", default-features = false, features = ["std"] }
+dvb-si   = { version = "11.0", features = ["ts", "serde", "chrono"] }
+dvb-t2mi = { version = "11.0", features = ["ts"] }
+mpeg-ts  = { version = "0.5", default-features = false, features = ["std"] }
```

## Fixes

- `pids` bitrate estimate (#1100). It mixed the first PCR seen on any PID with the last PCR seen on any, possibly different, PID. A multiplex carries one independent PCR clock per program (ISO/IEC 13818-1 §2.4.4.9), so that comparison is meaningless, and the command reported "n/a" on any capture with more than one PCR PID. It now picks one PCR PID and uses only that PID's first and last PCR. It also unwraps the PCR's own roll-over (`2^33 * 300` ticks of 27 MHz, about 26.5 hours) across samples on the selected PID, so a capture that crosses the wrap no longer reports "n/a" either.
- `services` (#1100). Services and logical channel numbers are now keyed by `(original_network_id, transport_stream_id, service_id)` instead of `service_id` alone. `service_id` is only unique within one transport stream (EN 300 468 §5.2.2/§5.2.3), so the same `service_id` on a second transport stream (SDT-other, or a second NIT entry) used to overwrite the first service and could be shown with the wrong LCN.
- `t2mi --inner` without `--plp` no longer corrupts user packets that span a BBFrame boundary when several PLPs interleave: see the `inner_ts::InnerTsRecovery` fix in the [dvb-t2mi 11.0.0](dvb-t2mi-11.0.0.md) note (#1034).

---

Published from tag `dvb-tools-v11.0.0`.
