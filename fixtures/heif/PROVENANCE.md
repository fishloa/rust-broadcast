# HEIF still-image fixtures

Written by the tools named below from one frame of the committed capture
`fixtures/mp4/h264-aac-prog.mp4`. The frame was extracted to a 160x120 PNG in a
scratch directory (not committed); the first-edition files are under 4 kB and the third-edition ones under 20 kB.

Tools:

| Tool | Version |
|---|---|
| `ffmpeg` | 8.1.2 |
| `heif-enc` / `heif-info` (libheif) | 1.23.1 |
| `MP4Box` (GPAC) | 26.07-revrelease |
| `sips` (macOS) | sips-316 |
| `avifenc` / `avifdec` (libavif, aom 3.14.1) | 1.4.2 |

Source frame:

```sh
ffmpeg -v error -y -i fixtures/mp4/h264-aac-prog.mp4 -frames:v 1 \
       -vf scale=160:-2 frame.png
ffmpeg -v error -y -i frame.png \
       -vf "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='255*(X/W)'" alpha.png
```

## `plain.heic`

```sh
heif-enc -q 60 -o fixtures/heif/plain.heic frame.png
```

`heif-info plain.heic`:

```
MIME type: image/heic
main brand: heic
compatible brands: mif1, heic, miaf
image: 160x120 (id=1), primary
  bit depth: 8
  color profile: nclx (primaries 1, transfer 13, matrix 6, full range)
```

`MP4Box -disox` of the item boxes:

```
<PrimaryItemBox Type="pitm" item_ID="1">
<ItemLocationBox Version="0" offset_size="4" length_size="4" base_offset_size="4" index_size="0">
  item_ID=1 data_reference_index=0 base_offset=378 construction_method=0
  extent_offset=0 extent_length=2093
<ItemInfoEntryBox Type="infe" Version="2" Flags="0" item_ID="1" item_type="hvc1">
ipco: hvcC, colr(nclx 1/13/6), ispe(160x120), pixi(8,8,8)
ipma: item 1 -> 1 essential, 2, 3, 4
```

## `thumb.heic`

```sh
heif-enc -q 60 -t 80 -o fixtures/heif/thumb.heic frame.png
```

Two items. `MP4Box -disox`:

```
pitm item_ID=1
iloc item 1 base_offset=629  extent 0+2093
     item 2 base_offset=2722 extent 0+1080
infe 1/2 item_type=hvc1
iref child thmb from_item_id=2 -> 1
ipco: hvcC, colr, ispe(160x120), pixi, hvcC(thumb), ispe(80x64), clap
ipma: item 1 -> 1e,2,3,4 ; item 2 -> 5e,2,6,4,7e
```

`heif-info` reports the thumbnail as 80x64 and the alpha channel as absent.

## `alpha.heic`

```sh
heif-enc -q 60 -o fixtures/heif/alpha.heic alpha.png
```

Two items, the second an auxiliary alpha plane: `iref` child `auxl` from item 2
to item 1, and its `auxC` property is
`urn:mpeg:hevc:2015:auxid:1` (ISO/IEC 23008-12:2017 B.2.3.3). `heif-info`
reports `alpha channel: yes`.

## `sips.heic`

```sh
sips -s format heic frame.png --out fixtures/heif/sips.heic
```

`MP4Box -disox` / `heif-info` show a file-level `meta` with `dinf`/`dref`,
`pitm` 1, one `hvc1` item, and two properties beyond the usual four: `clli`
(MaxCLL 203, MaxFALL 64) and `irot` (angle 0). It tests a `dinf` child and the
`clli`/`irot` item properties.

## `collection.mp4`

```sh
heif-enc -q 60 -o plain.heic frame.png
avifenc -q 60 -s 8 frame.png plain.avif
printf 'Exif\0\0MM\0*\0\0\0\x08\0\0\0\0\0\0' > exif.bin
printf '<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"></x:xmpmeta><?xpacket end="w"?>' > meta.xmp
MP4Box -add-image plain.heic:id=1:primary \
       -add-image plain.avif:id=2:group=altr,1 \
       -add-item exif.bin:type=Exif:id=3:name=exif:ref=cdsc,1 \
       -add-item meta.xmp:type=mime:mime=application/rdf+xml:id=4:name=xmp:ref=cdsc,1 \
       -new fixtures/heif/collection.mp4
```

`MP4Box -info`:

```
# File Meta type: "pict" - 4 resource item(s)
Primary Item - ID 1
Item #1: ID 1 type hvc1 size 160x120 3 channels (8 bpc)
Item #2: ID 2 type av01 size 160x120 3 channels (8 bpc)
Item #3: ID 3 type Exif Name "exif"
Item #4: ID 4 type mime Name "xmp" MIME: "application/rdf+xml"
```

The `grpl` box carries one `altr` group (id 1) with entity 2; both metadata
items have a `cdsc` reference to item 1. The file's major brand is `isom`.

## Payload extraction

`MP4Box -dump-item <id>:path=<file>` was used to confirm each item's payload
length. For HEVC items GPAC prepends the `hvcC` parameter sets to the dumped
bytes, so the payload hash used by the tests is taken over the byte range the
tool's own `iloc` dump states (`base_offset` + `extent_offset`, length
`extent_length`); the tests assert those ranges and their FNV-1a 64 values.

## Third-edition (2025) fixtures

The four files below exercise the item properties and entity groups the third
edition of ISO/IEC 23008-12 and Amendment 1 add. Written by the same tools and
from the same 160x120 frame as above (`heif-enc` 1.23.1, `avifenc` 1.4.2,
`MP4Box` 26.07-revrelease).

### `props.heic`

```sh
heif-enc -q 60 --pitm-description "Golden hour sample" --pasp 1,1 \
         --clli 400,200 --rotate-cw 90 --flip-h \
         -o fixtures/heif/props.heic frame.png
```

`heif-info props.heic`:

```
image: 120x160 (id=1), primary
  content light level: MaxCLL=400 MaxFALL=200
transformations:
  angle (ccw): 270
  mirror: horizontal
properties:
  user description:
    lang: 
    name: 
    description: Golden hour sample
    tags: 
  content light level (clli):
    max content light level: 400
    max pic average light level: 200
```

The item carries `udes` (6.5.20, description only), `pasp` (6.5.4), `clli`
(6.5.14), `irot` (6.5.10) and `imir` (6.5.12). `MP4Box -disox` calls `udes`
unknown in `ipco`, which is why `heif-info` is the independent dump for it.

### `tiled.heic`

```sh
heif-enc -q 60 --cut-tiles 80 -o fixtures/heif/tiled.heic frame.png
```

`heif-info tiled.heic`:

```
image: 160x120 (id=1), primary
  tiles: 2x2, tile size: 80x80
```

The primary item is a `grid` derived item (6.6.2.3) whose `dimg` reference
lists four hidden `hvc1` tiles.

### `premult.heic`

```sh
heif-enc -q 60 --premultiplied-alpha -o fixtures/heif/premult.heic alpha.png
```

`heif-info premult.heic` reports `alpha channel: yes (premultiplied)`. The
file carries a `prem` item reference (10.2.3) from the colour item to the
alpha item, alongside `auxl` and `auxC`.

### `groups.mp4`

```sh
MP4Box -brand heic -ab mif1 -ab miaf \
       -add-image props.heic:id=10:primary:group=ster,1 \
       -add-image frame.avif:id=11:group=ster,1 \
       -add-image frame.avif:id=12:group=pymd,2 \
       -add-image frame.avif:id=13:group=pymd,2 \
       -add-image frame.avif:id=14:group=slid,3 \
       -add-image frame.avif:id=15:group=slid,3 \
       -add-image frame.avif:id=16:group=prgr,4 \
       -add-image frame.avif:id=17:group=prgr,4 \
       -new fixtures/heif/groups.mp4
```

`MP4Box -info groups.mp4`:

```
# File Meta type: "pict" - 9 resource item(s)
Primary Item - ID 10
Item #1: ID 10 type hvc1 size 160x120 3 channels (8 bpc)
Item #2: ID 11 type av01 size 160x120 3 channels (8 bpc)
Item #3: ID 12 type av01 size 160x120 3 channels (8 bpc)
Item #4: ID 13 type av01 size 160x120 3 channels (8 bpc)
Item #5: ID 14 type av01 size 160x120 3 channels (8 bpc)
Item #6: ID 15 type av01 size 160x120 3 channels (8 bpc)
Item #7: ID 16 type av01 size 160x120 3 channels (8 bpc)
Item #8: ID 17 type av01 size 160x120 3 channels (8 bpc)
```

The groups list is `ster` (group 1, entities 10 and 11), `pymd` (group 2,
12 and 13), `slid` (group 3, 14 and 15) and `prgr` (group 4, 16 and 17), each
with flags 0 and no trailing body. `heif-info` refuses the file because it has
no `moov`; `MP4Box -info` and the raw item inventory are the independent dumps.

No tool installed writes `tmap`, `cfen`, a region item, a mask item, a
capture-information property or a group body (`pymd` tile sizes, `rgpa` area);
those fixtures are clause-assembled in `isobmff/tests/it/item_layer_2025.rs`
and labelled there.

## `sequence.heic`

An image sequence (clause 7) written by libheif's sequence mode from four
48x48 frames of `testsrc2` with a 90-degree hue step, a 10 Hz timebase:

```sh
for i in 0 1 2 3; do
  ffmpeg -y -v error -f lavfi -i "testsrc2=s=48x48" -frames:v 1 \
         -vf "hue=h=$((i*90))" -pix_fmt yuv420p s$i.png
done
heif-enc -S -q 60 --timebase 10 -o fixtures/heif/sequence.heic s0.png s1.png s2.png s3.png
```

`MP4Box -info sequence.heic`:

```
Major Brand hevc - version 0 - compatible brands: mif1 heic miaf msf1 isom hevc
# Track 1 Info - ID 1 - TimeScale 10
Media Duration 00:00:00.400
Media Samples: 4 - CFR 10/sec
Media Type: pict:hvc1
```

The sample entry's `ccst` (`00 00 00 10 63 63 73 74 00 00 00 00 c0 00 00 00`)
states `all_ref_pics_intra` 1, `intra_pred_used` 1 and `max_ref_per_pic` 0.
The test asserts the brands, the `pict` handler, those three constraints, four
samples at one tick each in a timescale of 10 (summing to 2880 bytes) and that
every sample lies inside the file.
