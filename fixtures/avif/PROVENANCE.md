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

## Payload hashes

Every fixture's item payload range is taken from `MP4Box -disox` (its `iloc`
entry states offset and length); the tests assert the range and the FNV-1a 64
hash of the file bytes at that range. `avifdec --info` was used as the
independent decoder-side check of dimensions, bit depth, alpha and CLLI.

## Existing fixtures

No fixture committed before this directory carries a file-level `meta` with
`iloc`/`iinf` item boxes: a walk of `fixtures/` found the eleven item files in
`fixtures/heif/` and `fixtures/avif/` only.
