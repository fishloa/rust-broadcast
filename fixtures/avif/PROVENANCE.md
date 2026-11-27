# AVIF still-image fixtures

Written by `avifenc` 1.4.2 (libavif, aom 3.14.1) and `sips` (macOS, sips-316)
from one 160x120 PNG frame extracted from `fixtures/mp4/h264-aac-prog.mp4`
(the `ffmpeg` command is in `fixtures/heif/PROVENANCE.md`). Every file is under
3 kB.

## `plain.avif`

```sh
avifenc -q 60 -s 8 frame.png fixtures/avif/plain.avif
```

`MP4Box -disox`: `ftyp` major `avif`, compatible `avif mif1 miaf MA1A`;
`pitm` item 1; one `av01` item named "Color"; `iloc` v0 offset_size 4,
length_size 4, base_offset_size 0, extent 275+1933; `ipco` is `ispe(160x120)`,
`pixi(8,8,8)`, `av1C`, `colr(nclx 1/13/6)`; `ipma` item 1 -> 1, 2, 3 essential,
4. `avifdec --info` reports 160x120, 8-bit, YUV444, full range.

## `alpha.avif`

```sh
avifenc -q 60 -s 8 alpha.png fixtures/avif/alpha.avif
```

Two `av01` items ("Color" and "Alpha"), `iref` child `auxl` from item 2 to item
1, and an `auxC` property whose `aux_type` is
`urn:mpeg:mpegB:cicp:systems:auxiliary:alpha` (AVIF 4.1). `avifdec --info`
reports `Alpha: Not premultiplied`.

## `hdr10.avif`

```sh
avifenc -q 60 -s 8 -d 10 --clli 1000,400 frame.png fixtures/avif/hdr10.avif
```

Ten bits per channel (`pixi` 10,10,10) and a `clli` property (MaxCLL 1000,
MaxFALL 400), which `avifdec --info` also reports.

## `grid.avif` and `grid-2x1.avif`

```sh
avifenc -q 60 -s 8 --grid 2x2 frame.png fixtures/avif/grid.avif
avifenc -q 60 -s 8 --grid 2x1 frame.png fixtures/avif/grid-2x1.avif
```

The primary item is a `grid` item whose own body is eight bytes; its `dimg`
reference lists the tiles in row-major order. `grid.avif`: `ipma` item 1 ->
`ispe(160x120)`, `pixi`, `colr`; the four hidden tile items share
`ispe(80x64)`; the grid body is `00 00 01 01 00 a0 00 78` (version 0, flags 0,
rows_minus_one 1, columns_minus_one 1, 160x120). `grid-2x1.avif`: rows 1,
columns 2, two tiles at 80x120. `avifdec --info` reports 160x120 for both.

## `sips.avif`

```sh
sips -s format avif frame.png --out fixtures/avif/sips.avif
```

One `av01` item with `dinf` present, plus `clli` (203, 64) and `irot` (angle 0)
properties; `avifdec --info` reports the same CLLI and `irot`.

## `gainmap-multichannel.avif`, `gainmap-single.avif` and `gainmap-swapped.avif`

Written by `avifgainmaputil` 1.4.2 (libavif 1.4.2, aom 3.14.1) and `ffmpeg`
8.0 from two 64x64 `testsrc2` frames:

```sh
ffmpeg -f lavfi -i "testsrc2=s=64x64" -frames:v 1 -pix_fmt rgb24 base.png
ffmpeg -f lavfi -i "testsrc2=s=64x64" -frames:v 1 -vf "eq=brightness=0.2" -pix_fmt rgb48be alt.png
ffmpeg -f lavfi -i "testsrc2=s=64x64" -frames:v 1 -vf "lutrgb=r=val*1.6:g=val*1.6:b=val*1.6" -pix_fmt rgb48be alt1.png
avifenc -s 10 --cicp 9/16/9 -d 10 alt.png alt.avif
avifenc -s 10 --cicp 9/16/9 -d 10 -y 444 alt1.png alt1.avif
avifgainmaputil combine base.png alt.avif gainmap-multichannel.avif
avifgainmaputil combine --yuv-gain-map 400 base.png alt1.avif gainmap-single.avif
avifgainmaputil swapbase --ignore-profile --cicp 9/16/9 gainmap-multichannel.avif gainmap-swapped.avif
```

Each file has three items: a base `av01` (1), the `tmap` item (2) and the
gain map `av01` (3), with `iref` `dimg` 2 -> [1, 3], `grpl` `altr` [2, 1] and
`tmap` among the compatible brands. `tmap` item 2 is 142 bytes in the two
multichannel files and 62 bytes in the single-channel one.

`avifgainmaputil printmetadata gainmap-multichannel.avif`:

```
 * Base headroom:      0 (as fraction: 0/1)
 * Alternate headroom: 4 (as fraction: 4/1)
 * Gain Map Min:       R -4.31352 (as fraction: -4523049/1048576) G -4.31352 (as fraction: -4523049/1048576) B -4.31352 (as fraction: -4523049/1048576)
 * Gain Map Max:       R 7.03147 (as fraction: 3686517/524288)    G 7.03147 (as fraction: 3686517/524288)    B 7.03147 (as fraction: 3686517/524288)
 * Base Offset:        R 0.015625 (as fraction: 1/64)             G 0.015625 (as fraction: 1/64)             B 0.015625 (as fraction: 1/64)
 * Alternate Offset:   R 0.015625 (as fraction: 1/64)             G 0.015625 (as fraction: 1/64)             B 0.015625 (as fraction: 1/64)
 * Gain Map Gamma:     R 1 (as fraction: 1/1)                     G 1 (as fraction: 1/1)                     B 1 (as fraction: 1/1)
 * Use Base Color Space: False
```

`printmetadata` 1.4.2 formats the same channel for R, G and B, so it cannot
confirm channels 1 and 2. libultrahdr's own `uhdr_gainmap_metadata_frac` decoder
(Apache-2.0, `lib/src/gainmapmetadata.cpp`, built from source for this check)
read the `tmap` bodies independently:

```
gainmap-multichannel.avif  status 0  base 0/1 alt 4/1
  ch0 min -4523049/1048576 max 3686517/524288 gamma 1/1 boff 1/64 aoff 1/64
  ch1 min -8657321/4194304 max 13606797/2097152 gamma 1/1 boff 1/64 aoff 1/64
  ch2 min -4761391/2097152 max 889281/131072 gamma 1/1 boff 1/64 aoff 1/64
gainmap-single.avif  status 0  base 0/1 alt 4/1
  ch0 min -1161593/2097152 max 14152539/2097152 gamma 1/1 boff 1/64 aoff 1/64
  (printed for ch1/ch2 too; the flags byte is 0x00, so the file carries one)
gainmap-swapped.avif  status 0  base 4/1 alt 0/1, use base colour space set
  ch0 min -4523049/1048576 max 3686517/524288 gamma 1/1 boff 1/64 aoff 1/64
  ch1 min -8657321/4194304 max 13606797/2097152 gamma 1/1 boff 1/64 aoff 1/64
  ch2 min -4761391/2097152 max 889281/131072 gamma 1/1 boff 1/64 aoff 1/64
```

`gainmap-single.avif` printmetadata lines (the values the test asserts):

```
 * Base headroom:      0 (as fraction: 0/1)
 * Alternate headroom: 4 (as fraction: 4/1)
 * Gain Map Min:       -0.553891 (as fraction: -1161593/2097152), all three columns
 * Gain Map Max:       6.74846 (as fraction: 14152539/2097152), all three columns
 * Gain Map Gamma:     1/1,  Base Offset 1/64,  Alternate Offset 1/64
```

`gainmap-swapped.avif` printmetadata lines:

```
 * Base headroom:      4 (as fraction: 4/1)
 * Alternate headroom: 0 (as fraction: 0/1)
 * Gain Map Min:       -4.31352 (as fraction: -4523049/1048576)
 * Gain Map Max:       7.03147 (as fraction: 3686517/524288)
 * Gain Map Gamma:     1/1,  Base Offset 1/64,  Alternate Offset 1/64
 * Use Base Color Space: True
```

## Payload hashes

Every fixture's item payload range is taken from `MP4Box -disox` (its `iloc`
entry states offset and length); the tests assert the range and the FNV-1a 64
hash of the file bytes at that range. `avifdec --info` was used as the
independent decoder-side check of dimensions, bit depth, alpha and CLLI.

## Existing fixtures

No fixture committed before this directory carries a file-level `meta` with
`iloc`/`iinf` item boxes: a walk of `fixtures/` found the eleven item files in
`fixtures/heif/` and `fixtures/avif/` only.

## `sequence.avif`

An AVIF image sequence (`avis`, clause 7) written by `avifenc` from four frames
with a 10 Hz timescale:

```sh
avifenc -s 10 --timescale 10 f0.png f1.png f2.png f3.png fixtures/avif/sequence.avif
```

`MP4Box -info sequence.avif`:

```
Major Brand avis - version 0 - compatible brands: avif avis msf1 iso8 mif1 miaf MA1A
Movie Info - 1 track - TimeScale 10
# Track 1 Info - ID 1 - TimeScale 10
Media Duration 00:00:00.400
Media Samples: 4 - CFR 10/sec
Media Type: pict:av01
```

The sample entry's `ccst` (`00 00 00 10 63 63 73 74 00 00 00 00 7c 00 00 00`)
states `all_ref_pics_intra` 0, `intra_pred_used` 1 and `max_ref_per_pic` 15.
`avifenc` reports the colour payload as 4059 bytes; the test asserts the
brands, the `pict` handler, those three constraints, four samples at one tick
each in a timescale of 10 summing to 4059 bytes, and that every sample lies
inside the file.
