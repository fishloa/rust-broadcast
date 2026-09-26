# SCTE-35 fixtures — issue #1039 / #1040

Compiled with **TSDuck 3.44-4676** (`tstabcomp`) from synthetic input, released
under the workspace licence.

## `splice_insert_pts_adjustment.bin`

Oracle for #1039 (`pts_adjustment` ignored). Compiled from:

```xml
<tsduck>
  <splice_information_table pts_adjustment="8100000">
    <splice_insert splice_event_id="2002" out_of_network="true" pts_time="900000" unique_program_id="1" avail_num="0" avails_expected="0">
      <break_duration auto_return="true" duration="2160000" />
    </splice_insert>
  </splice_information_table>
</tsduck>
```

```
tstabcomp --compile splice_insert_pts_adjustment.xml
```

`tstabcomp --decompile splice_insert_pts_adjustment.bin`'s own output (the
independent-tool oracle asserted in `tests/scte35_pts_adjustment.rs`):

```
<splice_information_table protocol_version="0" pts_adjustment="8,100,000" tier="0x0FFF">
  <splice_insert splice_event_id="0x000007D2" ... out_of_network="true" ... pts_time="900,000">
    <break_duration auto_return="true" duration="2,160,000"/>
  </splice_insert>
</splice_information_table>
```

So the expected absolute cue time is `pts_time + pts_adjustment (mod 2^33)` =
`900,000 + 8,100,000 = 9,000,000` ticks (100.000s at 90 kHz).

## `time_signal_segmentation.bin`

Oracle for #1040 (`time_signal` cues lose time/kind/id). Compiled from:

```xml
<tsduck>
  <splice_information_table pts_adjustment="0">
    <time_signal pts_time="1234567" />
    <splice_segmentation_descriptor segmentation_event_id="0x4800000A" segmentation_type_id="0x22" segment_num="0" segments_expected="0" segmentation_duration="900000">
      <segmentation_upid type="0x0C">0011223344556677</segmentation_upid>
    </splice_segmentation_descriptor>
  </splice_information_table>
</tsduck>
```

```
tstabcomp --compile time_signal_segmentation.xml
```

`tstabcomp --decompile time_signal_segmentation.bin`'s own output:

```
<time_signal pts_time="1,234,567"/>
<splice_segmentation_descriptor segmentation_event_id="0x4800000A" ... segmentation_duration="900,000" segmentation_type_id="0x22" ...>
```

`segmentation_type_id = 0x22` is Table 23's `Break Start`. Expected:
`at = 1,234,567` ticks, `id = Some(0x4800000A)`, `kind = BreakStart`,
`duration = Some(900,000)` ticks.
