# dvb-tools test fixtures

## `sdt_nit_two_ts_shared_service_id.ts`

Synthetic capture built with TSDuck 3.44 to exercise the case where two
different transport streams each carry a service numbered `service_id =
0x0301` (ETSI EN 300 468 §5.2.2/§5.2.3 only guarantees `service_id`
uniqueness within one `(original_network_id, transport_stream_id)`
namespace) — audit finding W-DT-2 / issue #1100.

Contents: SDT actual for TS `(onid=0x0001, tsid=0x0006)` naming service
`0x0301` "Service A", SDT other for TS `(onid=0x0001, tsid=0x0007)` naming
the same numeric `service_id` "Service B", and a NIT actual listing both
transport streams, each with an `eacem_logical_channel_number_descriptor`
(tag 0x83, preceded by its `private_data_specifier_descriptor` = EACEM)
assigning LCN 101 to the first and LCN 202 to the second.

Generated from synthetic input only (no real broadcast captured); released
under the workspace licence.

```sh
cat > sdt_only.xml <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <SDT transport_stream_id="0x0006" original_network_id="0x0001" actual="true">
    <service service_id="0x0301" EIT_schedule="false" EIT_present_following="false" running_status="running" CA_mode="false">
      <service_descriptor service_type="0x01" service_provider_name="Synthetic" service_name="Service A"/>
    </service>
  </SDT>
  <SDT transport_stream_id="0x0007" original_network_id="0x0001" actual="false">
    <service service_id="0x0301" EIT_schedule="false" EIT_present_following="false" running_status="running" CA_mode="false">
      <service_descriptor service_type="0x01" service_provider_name="Synthetic" service_name="Service B"/>
    </service>
  </SDT>
</tsduck>
XML

cat > nit_only.xml <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <NIT network_id="0x0001" actual="true">
    <transport_stream transport_stream_id="0x0006" original_network_id="0x0001">
      <private_data_specifier_descriptor private_data_specifier="eacem"/>
      <eacem_logical_channel_number_descriptor>
        <service service_id="0x0301" logical_channel_number="101" visible_service="true"/>
      </eacem_logical_channel_number_descriptor>
    </transport_stream>
    <transport_stream transport_stream_id="0x0007" original_network_id="0x0001">
      <private_data_specifier_descriptor private_data_specifier="eacem"/>
      <eacem_logical_channel_number_descriptor>
        <service service_id="0x0301" logical_channel_number="202" visible_service="true"/>
      </eacem_logical_channel_number_descriptor>
    </transport_stream>
  </NIT>
</tsduck>
XML

tstabcomp --compile sdt_only.xml -o sdt_only.bin
tstabcomp --compile nit_only.xml -o nit_only.bin
tsp -I null 1000 \
    -P inject sdt_only.bin --pid 0x11 --inter-packet 50 \
    -P inject nit_only.bin --pid 0x10 --inter-packet 50 \
    -O file sdt_nit_two_ts_shared_service_id.ts
```

(TSDuck 3.44-4676, `tsp`/`tstabcomp`.)

## `france-tnt-pcr.ts` (workspace root `fixtures/`, referenced by
`pids_bitrate_oracle.rs`)

Pre-existing real broadcast capture (see workspace root `fixtures/` for its
own provenance); reused here as the independent oracle for `dvb-tools pids`
bitrate estimation (audit finding W-DT-1 / issue #1100) because it carries
five independent PCR PIDs (0x0078, 0x00DC, 0x026C, 0x0208, 0x02D0), one per
service.

## `france-tnt-pcr-wrap.ts`

Derived from `france-tnt-pcr.ts` to exercise the PCR-wrap follow-up to
W-DT-1 (issue #1100): PID 0x00DC isolated alone (so no *other* PCR PID can
mask the bug by giving the PID-selection logic a still-valid fallback
candidate), then its PCR values shifted with TSDuck's `pcredit --add-pcr`
(which performs correct modular PCR arithmetic on the 27 MHz clock) so they
cross the wrap partway through the file. `--add-pcr`'s value is `(2^33 *
300) - 1_500_000 - <first PCR of PID 0x00DC in france-tnt-pcr.ts>` — i.e.
the shift that lands the first sample 1,500,000 ticks before the wrap.

```sh
# Isolate PID 0x00DC (its own independent PCR clock, ISO/IEC 13818-1
# §2.4.4.9) from the original capture.
tsp -I file france-tnt-pcr.ts -P filter --pid 0x00DC -O file single_pid.ts

# First PCR on that PID (from `tsp -P pcrextract --pcr -p 0x00DC`):
# 920154241112. Shift so it lands near the top of the 2^33*300 modulus:
tsp -I file single_pid.ts \
    -P pcredit --add-pcr 1656824636488 \
    -O file france-tnt-pcr-wrap.ts
```

(TSDuck 3.44-4676, `tsp`.) Oracle for this fixture (before the shift, since
`pcredit --add-pcr` only rewrites PCR fields, not packet count/size/timing):
`tsanalyze single_pid.ts` reports `Selected reference bitrate: 4,113,084
b/s` (188 bytes/pkt).
