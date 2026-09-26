# `scte35-other-pid.ts` — SCTE-35 cue on a non-default PID

For issue #1046 (`media-doctor` audit MD-C1): `Scte35Check` hard-coded PID
`0x01F0`, so any real capture carrying its cues on a different PID (the norm
— SCTE-35 is carried on whichever PID the PMT declares with `stream_type
0x86`, never a fixed one) got zero findings.

Synthetic, generated with TSDuck 3.44 (`tsp`/`tstabcomp`) — a real PSI/SI
compiler and injector, not hand-rolled bytes. 400 null-filled TS packets (one
program), with:

- PAT (PID `0x0000`) declaring one service (`service_id=1`), PMT PID
  `0x1000`.
- PMT (PID `0x1000`) declaring PID `0x0100` (`stream_type=0x02`, MPEG-2
  video, unused filler) and PID **`0x0150`** (`stream_type=0x86`) with a
  `registration_descriptor` (`format_identifier=0x43554549`, `"CUEI"`) — the
  spec-correct way a real encoder signals the SCTE-35 PID.
- A `splice_information_table` / `splice_insert` (`splice_event_id=777`,
  `out_of_network=true`, `splice_immediate=true`) injected on PID `0x0150`
  only — deliberately **not** `0x01F0`.

Regenerate:

```bash
cat > pat.xml <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <PAT transport_stream_id="1">
    <service service_id="1" program_map_PID="0x1000"/>
  </PAT>
</tsduck>
EOF
cat > pmt.xml <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <PMT service_id="1" PCR_PID="0x100">
    <component stream_type="0x02" elementary_PID="0x100"/>
    <component stream_type="0x86" elementary_PID="0x150">
      <registration_descriptor format_identifier="0x43554549"/>
    </component>
  </PMT>
</tsduck>
EOF
cat > splice.xml <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <splice_information_table>
    <splice_insert splice_event_id="777" out_of_network="true"
                    splice_immediate="true" unique_program_id="1"/>
  </splice_information_table>
</tsduck>
EOF
tstabcomp --compile pat.xml -o pat.bin
tstabcomp --compile pmt.xml -o pmt.bin
tstabcomp --compile splice.xml -o splice.bin

tsp -I null 400 \
  -P inject pat.bin    -p 0x0000 --inter-packet 50 \
  -P inject pmt.bin    -p 0x1000 --inter-packet 50 \
  -P inject splice.bin -p 0x0150 --inter-packet 50 \
  -O file scte35-other-pid.ts
```

Verify with `tsanalyze scte35-other-pid.ts`: PID `0x0150` reports as
`SCTE 35 Splice Info`, PMT PID `0x1000`.

Synthetic input only (no third-party media); released under the workspace
licence (MIT OR Apache-2.0).
