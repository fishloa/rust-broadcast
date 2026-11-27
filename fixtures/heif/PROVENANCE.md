# HEIF still-image fixtures

Written by the tools named below from one frame of the committed capture
`fixtures/mp4/h264-aac-prog.mp4`. The frame was extracted to a 160x120 PNG in a
scratch directory (not committed); every file here is under 4 kB.

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
