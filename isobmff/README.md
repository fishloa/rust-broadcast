# isobmff

ISO/IEC 14496-12:2026 (ISO Base Media File Format) **box syntax**, no_std +
`alloc`, depending only on `broadcast-core`.

This crate is the box layer below the `transmux` muxing hub: it parses and
writes the `size`/`type` box headers (including the 64-bit `largesize` and the
`uuid` extended type), the top-level and movie-fragment box set, the `moov`
tree and its sample entries, the sample-table/timing boxes, the
visual-extension boxes, the sample-group boxes, the CENC box syntax
(ISO/IEC 23001-7), and the DASH Event Message Box (`emsg`, ISO/IEC 23009-1).
It never knows the media IR or a codec bitstream.

## Coverage

Typed 167 of 167 registry box types (ISO/IEC 14496-12:2026). The
`tests/it/coverage.rs` test loads
`private/specs-verified/mpeg/isobmff/box-registry-14496-12-2026.tsv` and pins
every remaining type under its issue tier, so these counts cannot drift; the
types themselves are the one table in `src/coverage.rs`.

| Module | Boxes (four-CC) |
|---|---|
| `box_types` | `size`/`type` header, `largesize`, `uuid` usertype, the `BoxIter` walker |
| `box_support` | shared opaque-child and container building helpers |
| `segments` | `ftyp`, `styp`, `mdat`, `imda`, `tyco`, `etyp`, `otyp` |
| `init_segment` | `moov`, `mvhd`, `trak`, `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `vmhd`/`smhd`/`nmhd`/`hmhd`/`vvhd`, `dinf`/`dref`, `stbl` (`stts`/`ctts`/`stsc`/`stsz`/`stz2`/`sdtp`/`stco`/`co64`/`stss`), `stsd`, `edts`/`elst`, `mvex`/`trex`/`mehd`, the audio sample entries |
| `sample_entries` | `avc1`/`avc3`, `hvc1`/`hev1`, `vvc1`/`vvi1`, `mp4v`, `btrt`, `VisualSampleEntry` |
| `subtitle_entries` | `stpp`, `wvtt`, `vttc`/`vtte`/`vtta`/`payl`/`sttg`/`iden` |
| `avc_parameter_set` | `avcp` sample entry and parameter set sample |
| `track_reference` | `tref` |
| `track_id` | `track_ID` newtype (ISO/IEC 14496-12:2026 §8.3.2) |
| `avc_config` | `avcC` |
| `hevc_config` | `hvcC` |
| `vvc_config` | `vvcC` |
| `nalu_types` | `AvcSps`/`AvcPps`/`HevcNalUnit` newtypes over parameter-set NAL units |
| `mp4esds` | `esds` (ES/DecoderConfig/DecoderSpecificInfo/SLConfig descriptors) |
| `ac3` | `dac3`, `dec3` |
| `ac4` | `dac4` (`ac-4` entry) |
| `av1` | `av1C`, `av01` |
| `vp9` | `vpcC`, `vp09` |
| `opus` | `dOps`, `Opus` |
| `flac` | `dfLa`, `fLaC` |
| `dts` | `ddts`, `dtsc`/`dtsh`/`dtsl`/`dtse` |
| `mpegh` | `mhaC`, `mha1`/`mha2`/`mhm1`/`mhm2` |
| `dovi` | `dvcC`, `dvvC` |
| `avs2` | `av2c`, `la2c`, `avst`, `lav2` |
| `avs3` | `av3c`, `lavc`, `avs3`, `lav3`, plus its sample-group entries |
| `avs3_audio` | `dca3`, `av3a` |
| `hdr_boxes` | `mdcv`, `clli` |
| `timing` | `stts`, `ctts`, `cslg`, `elst`, `sidx` |
| `visual_ext` | `pasp`, `clap`, `colr`, `amve`, `cclv` |
| `audio_boxes` | `chnl`, `dmix`, `ludt`/`tlou`/`alou`, `labl` |
| `audio_elements` | `ardi`, `aelm`/`aedb`/`aepp`/`aepr`, `aesb`/`aesd` |
| `track_desc` | `elng`, `kind`, `trgr` (`msrc`/`ster`), `tsel`, `stvi`, `strk` (`stri`/`strd`) |
| `index_boxes` | `ssix`, `leva`, `pdin`, `trep`, `assp` |
| `sample_tables_extra` | `stdp`, `padb`, `stsg`, `stsh` |
| `cenc` | `tenc`, `senc`, `pssh`, `saiz`, `saio`, `sinf`, `frma`, `schm`, `schi` (with `spki`) |
| `scheme` | `rinf`, `csch`, `spki`, `scrb`, `cinf` |
| `sample_groups` | `prft`, `sgpd`, `sbgp`, `csgp`, `subs`, `ssrt` |
| `movie_fragment` | `moof`, `mfhd`, `traf`, `tfhd`, `tfdt`, `trun`, `rsot` |
| `limits` | hostile-input caps (`Limits`) for the Phase 2 reader |
| `writer` | sans-IO fragmented writer (init segment, `moof`+`mdat`, CENC media-segment protection, recorded/compact fragment layouts, regenerated `sidx`/`mfra`) and progressive writer (`Writer::progressive`: whole-file layout from a plan, `stco`/`co64`, seekable or append-only sink) |
| `sample_table` | lazy per-track sample cursor (`TrackCursor`/`CursorState`) and the run-length table builders |
| `reader` | sans-IO progressive-file `Reader`: asks for structure byte ranges, yields every track and one sample descriptor at a time, `seek_to` over `stss` |
| `plan` | `SamplePlan`, the progressive index a `Reader` hands out without reading media; `OwnedPlan` and `PlanBuilder`, its owned form; `MovieShell`/`TopLevelBox`, the boxes a plan carries back to a writer |
| `frag_offsets` | ISO/IEC 14496-12:2026 §8.8.7/§8.8.8 sample-data addressing rules |
| `frag_layout` | `FragmentLayout`: the `tfhd`/`trun` field choices, run splits and child order a fragment states, plus the `sidx`/`mfra` forms around it — `reader::FragmentInfo` reports one and `FragmentedWriter::set_fragment_layout` reproduces it byte for byte |
| `reader` (fragmented) | the same `Reader` reads a fragmented file one `moof` at a time and seeks through `sidx` or the trailing `tfra` |
| `reader` (permissive) | `Mode::Permissive` reports the design §4.7 faults as `Step::Warning`; strict mode keeps rejecting them |
| `quicktime` | QuickTime atoms, read and written: sound description versions 1/2, `wave`, `gmhd`, `tmcd`, `tapt`, chapter titles, `qt  `/brandless files |
| `driver` | blocking drivers over the reader: `FileReader` (`Read + Seek`) and `ForwardReader` (`Read` only), with a bounded reusable buffer |
| `timing` (random access) | `tfra`, `mfra`, `mfro` |
| `emsg` | `emsg` (versions 0 and 1, ISO/IEC 23009-1 §5.10.3.3) |
| `metadata` | `udta` (with `cprt`/`strk`/`hnti`/`hinf`), `meta` (with `xml `/`bxml`/`fiin`), `hdlr`, `keys`, `ilst`, `data`, `mean`, `name`, `chpl`, `loci`; QuickTime user-data text and `©xyz` ISO 6709 |
| `meta_track` | `urim` (`uri `/`uriI`), `metx`, `mett`/`stxt` (`txtC`), `mebx` (`keys`/`keyd`/`loca`/`setu`), `mime`, `xml `/`bxml`, `cprt` |
| `item` | ISO/IEC 14496-12:2026 §8.11/§8.15 item boxes: `iloc`, `iinf`/`infe`, `pitm`, `iref` and its typed reference boxes, `iprp`/`ipco`/`ipma`, `idat`, `ipro`, `grpl` with `EntityToGroupBox`, and the `brnd`/`hdlp` properties |
| `image` | ISO/IEC 23008-12:2025 clause 6 image properties (`ispe`, `pixi`, `rloc`, `auxC`, `irot`, `lsel`, `imir`), the derived bodies `grid`/`iovl`/`iden`, the AVIF `a1op`/`a1lx` properties and brands; `pasp`/`colr`/`clap`/`hvcC`/`avcC`/`av1C`/`mdcv`/`clli` are the crate's existing types |
| `image_props` | the clause 6.5 properties the 2025 edition and Amendment 1 add (`iscl`, `rref`, `crtt`, `mdft`, `udes`, `altt`, the capture-information set, `tols`, the transitions and their timings, `prdi`, `sstr`, `cmex`, `cmin`, `cexg`, `dadj`, `stag`, `reve`, `ndwt`, `mskC`) |
| `image_derived` | the `tmap` (tone map, ISO 21496-1 metadata typed on the open-implementation layout) and `cfen` (colour format enhancement) derived bodies |
| `image_groups` | typed entity-group bodies: `pymd` tile sizes and `rgpa` area rectangles |
| `image_brands` | clause 10 brands, reader-property tables and structurally checkable findings |
| `image_sequence` | clause 7 boxes: `ccst`, `auxi`, the `refs` sample group |
| `region` | clause 11 regions: `RegionGeometry`, `RegionItem`, mask configuration, region-track config/sample, `regm` and `udes` sample groups |
| `integrity` | clause 8.4 `md5i` and the `mint` item body |
| `item_reader` | sans-IO still-image reader: asks for the top-level headers, the file-level `meta` and the tiny derived-item bodies, then yields `StillImage` (brands, primary item, every item with properties/references/data ranges, entity groups) plus a re-emit plan |
| `item_writer` | builds one still-image file from typed parts (brands, items, properties, references, primary item, entity groups); it chooses the `iloc` field widths and every offset |
| `hint` | `hnti`/`rtp `/`sdp `, `hinf` counters, `tims`/`tsro`/`snro`/`tssy`/`rssr`/`srpp`/`rtpx`/`rcsr`, `tPAT`/`tPMT`/`tsti`/`istm`, the RTP/MPEG-2 TS hint sample entries and the RTP sample format |
| `file_delivery` | `fiin`/`paen`/`fpar`/`fecr`/`fire`/`segr`/`gitn`, `fdsa`/`fdpa`/`extr`/`feci`, the FD hint sample entry and the FD sample format; the `fdel` item information extension lives in `item` |
| `registry` | runtime registration of private boxes and sample entries (`BoxDef`, `BoxRegistry`, `RegisteredEntry`) |

Every type implements the workspace `Parse` / `Serialize` pair, so a parse →
serialize round-trip is byte-identical.

## Tier 1 box evidence

The tier 1 boxes (issue #4167) are proven one of two ways: against bytes an
independent tool wrote, or against bytes assembled from the clause. A
fixture-backed box is asserted field by field against that tool's own dump and
re-serialized byte-exact from the file's bytes; a clause-assembled box says so
in its test. `fixtures/mp4/boxes/PROVENANCE.md` records the exact command, the
tool version and the dump for every file, and lists what was tried for each box
that no installed tool writes.

| Box | Clause | Evidence | File |
|---|---|---|---|
| `chnl` | §12.2.4 | real: written by ffmpeg 8.1.2, dumped by GPAC 26.07 | `mp4/boxes/chnl-ipcm.mp4` |
| `dmix` | §12.2.5 | clause-assembled | — |
| `ludt` | §12.2.7 | real: Apple `afconvert --soundcheck-generate` | `mp4/boxes/loudness.m4a` |
| `tlou` | §12.2.7 | real: same file | `mp4/boxes/loudness.m4a` |
| `alou` | §12.2.7 | clause-assembled | — |
| `labl` | §8.10.5 | clause-assembled | — |
| `elng` | §8.4.6 | real: GPAC 26.07 | `mp4/boxes/track-description.mp4` |
| `kind` | §8.10.4 | real: GPAC 26.07 | `mp4/boxes/track-description.mp4` |
| `trgr` | §8.3.4 | real: GPAC 26.07 | `mp4/boxes/track-description.mp4`, `mp4/boxes/track-group-ster.mp4` |
| `tsel` | §8.10.3 | real: GPAC 26.07, dumped by `MP4Box -disox` | `mp4/boxes/tsel.mp4` |
| `stvi` | §13.5.4.2 | clause-assembled | — |
| `ssix` | §8.14.4 | real: GPAC 26.07 `-dash -ssix` | `mp4/boxes/ssix-sidx.mp4` |
| `leva` | §8.8.13 | clause-assembled | — |
| `pdin` | §8.1.3 | clause-assembled | — |
| `trep` | §8.8.15 | clause-assembled | — |
| `csgp` | §8.9.5 | clause-assembled | — |
| `stdp` | §8.5.3 | clause-assembled | — |
| `padb` | §8.7.6 | clause-assembled | — |
| `stsg` | §8.13.6 | clause-assembled | — |

## Tier 5 box evidence

The last tier of issue #4167 closes the hint-track and file-delivery families.
`tests/it/tier5_fixtures.rs` asserts the boxes a real tool wrote against that
tool's dump and re-serializes each byte-exact; `tests/it/tier5_boxes.rs` holds
the clause-assembled ones. `fixtures/mp4/boxes/PROVENANCE.md` records the
commands and dumps, and the walk of all 252 committed MP4-family files.

| Box | Clause | Evidence | File |
|---|---|---|---|
| `btrt` | §8.5.2 | real: libavformat 8.1.2 (91 fixtures carry it) | `mp4/h264-aac-prog.mp4` |
| `hnti`, `rtp `, `sdp ` | §9.1.4 | real: MP4Box 26.07 and ffmpeg 8.1.2 `-movflags rtphint` | `mp4/boxes/hint-*.mp4` |
| `tims`, `tsro`, `snro` | §9.1.2 | real: MP4Box 26.07 `-hint -static -no-offset` | `mp4/boxes/hint-offsets.mp4` |
| `hinf` and its 15 counters | §9.1.5 | clause-assembled | — |
| `tPAT`, `tPMT`, `tsti`, `istm` | §9.3.3.2 | clause-assembled | — |
| `tssy`, `rssr`, `srpp`, `rtpx`, `rcsr` | §9.4.1, §9.1.2.2 | clause-assembled | — |
| `rtp ` sample entry, RTP sample format | §9.1.2, §9.1.3 | real entry + clause sample | `mp4/boxes/hint-*.mp4` |
| `srtp`, `rrtp`, `rsrp`, `rtcp`, `stcp`, `prtp` entries | §9.1.2, §9.4 | clause-assembled | — |
| `rm2t`, `sm2t`, `pm2t` entries | §9.3.3, §9.3.5 | clause-assembled | — |
| `fiin`, `paen`, `fpar`, `fecr`, `fire`, `segr`, `gitn` | §8.12 | clause-assembled | — |
| `fdp ` entry, `fdsa`, `fdpa`, `extr`, `feci`, FD sample format | §9.2 | clause-assembled | — |
| `fdel` item extension | §8.11.6.1 | clause-assembled | — |
| `stsh` | §8.6.3.2 | clause-assembled | — |
| `rsot` | §8.8.18 | clause-assembled | — |

No registry type is outside a box, so there is no `NOT_A_BOX` list: all 167
registry entries are boxes and all 167 are typed.

## Registered codes (MP4RA)

The MP4 Registration Authority lists every registered four-character code with the
specification that defines it. `tests/it/mp4ra.rs` loads all 30 tables from
`private/specs-verified/mpeg/isobmff/mp4ra/` and partitions every code into four
lists: **typed** (the crate parses it to a typed value, proven in `src/coverage/mp4ra.rs`),
**held, untyped** (work to do: the defining document is under `private/specs/` or
transcribed under `private/specs-verified/`), **not held** (the defining document is not
held, so implementing it would be guessing), and **not applicable** (not an ISOBMFF
structure this crate parses). A code in no list, or in two, fails the test.

| Table | Codes | Typed | Held, untyped | Not held | Not applicable |
|---|---:|---:|---:|---:|---:|
| `aux-info-types.csv` | 7 | 2 | 0 | 5 | 0 |
| `boxes.csv` | 281 | 130 | 23 | 112 | 16 |
| `boxes-qt.csv` | 16 | 7 | 0 | 2 | 7 |
| `boxes-udta.csv` | 42 | 24 | 0 | 18 | 0 |
| `brands.csv` | 291 | 95 | 0 | 196 | 0 |
| `checksum-types.csv` | 2 | 0 | 0 | 0 | 2 |
| `color-types.csv` | 5 | 4 | 0 | 1 | 0 |
| `data-references.csv` | 4 | 4 | 0 | 0 | 0 |
| `entity-groups.csv` | 29 | 20 | 0 | 9 | 0 |
| `handlers.csv` | 49 | 17 | 0 | 32 | 0 |
| `item-properties.csv` | 85 | 48 | 1 | 36 | 0 |
| `item-references.csv` | 19 | 18 | 0 | 1 | 0 |
| `item-types.csv` | 27 | 21 | 0 | 6 | 0 |
| `key-namespaces.csv` | 4 | 4 | 0 | 0 | 0 |
| `knownduplicates.csv` | 0 | 0 | 0 | 0 | 0 |
| `multiview-attributes.csv` | 14 | 4 | 10 | 0 | 0 |
| `oti.csv` | 55 | 0 | 0 | 0 | 55 |
| `sample-entries.csv` | 191 | 103 | 1 | 87 | 0 |
| `sample-entries-boxes.csv` | 100 | 32 | 16 | 52 | 0 |
| `sample-entries-qt.csv` | 59 | 45 | 0 | 14 | 0 |
| `sample-groups.csv` | 67 | 44 | 0 | 23 | 0 |
| `schemes.csv` | 24 | 5 | 0 | 19 | 0 |
| `stream-types.csv` | 15 | 0 | 0 | 0 | 15 |
| `textualcontent.csv` | 25 | 0 | 0 | 0 | 25 |
| `track-groups.csv` | 10 | 2 | 0 | 8 | 0 |
| `track-references.csv` | 44 | 23 | 0 | 21 | 0 |
| `track-references-qt.csv` | 5 | 5 | 0 | 0 | 0 |
| `track-selection.csv` | 18 | 17 | 0 | 1 | 0 |
| `uncv-profiles.csv` | 17 | 0 | 0 | 0 | 17 |
| `unlisted.csv` | 61 | 45 | 1 | 9 | 6 |
| **Total** | 1566 | 719 | 52 | 652 | 143 |

Trailing spaces are trimmed in these tables: a four-CC such as `rap ` is shown as
`rap` and space-padded back to four bytes for its proof. Duplicate CSV rows (one code
listed twice, e.g. `xml ` under ISO and JPEG 2000) count once.

A `NALu Video` code added after the held ISO/IEC 14496-15:2017 edition (VVC, EVC and
the neural-network codes) is counted as not held even though the family name matches.

### Specifications not held

Ordered by how many registered codes each would unlock (codes this crate does not
type). Buying or finding one of these documents is what turns a pinned code into work.

| Specification | Codes |
|---|---:|
| OMAF | 80 |
| NALu Video | 43 |
| CMAF | 41 |
| V3C-SYS | 35 |
| UNCV | 32 |
| Apple | 32 |
| MP4v2 | 23 |
| JPEG2000 | 17 |
| DMB-MAF | 16 |
| JPXS | 15 |
| ISO-Partial | 15 |
| DVB | 15 |
| CENC | 13 |
| OMA DRM 2.0 | 13 |
| IFE | 13 |
| DRC | 11 |
| MIAF | 10 |
| Sony | 9 |
| ISO-Text | 9 |
| JPEG XL | 8 |
| DECE | 7 |
| J2KHEIF | 7 |
| Metrics | 6 |
| Digital Voodoo | 6 |
| Dolby | 5 |
| JPEGAI | 5 |
| BitJazz | 5 |
| OMA DRM 2.1 | 4 |
| 3GPP2 | 4 |
| 3GPP-VR | 4 |
| iTunes | 4 |
| SAVX | 4 |
| DTS-UHD | 4 |
| MJ2 | 4 |
| MPEG-H | 4 |
| NGA.STND.0076 | 3 |
| Hipix | 3 |
| SDV | 3 |
| Youtube | 3 |
| Canon | 3 |
| SL-HDR | 3 |
| OGC 24-038 | 3 |
| ISO-UNCA | 3 |
| VPxx | 3 |
| ISO Variants | 3 |
| id3v2 | 2 |
| ONVIF | 2 |
| Samsung | 2 |
| T-AI-109.7 | 2 |
| ATSC 3.0 A345 | 2 |
| SCTE-215-1-1b | 2 |
| AOM-IAMF | 2 |
| MPEG-21 | 2 |
| OMA DRM XBS | 2 |
| Deprecated | 2 |
| Auro | 2 |
| OpenAPV | 2 |
| Dirac | 2 |
| Dolby MLP | 2 |
| SCL6 | 2 |
| Universal Music Group | 1 |
| ATSC 3.0 A337 | 1 |
| JPEG Systems | 1 |
| WhatsApp | 1 |
| MPEG-VSAF | 1 |
| DASH-IF watermarking | 1 |
| OMArlin | 1 |
| AppleCTLC | 1 |
| 3GPP-DASH | 1 |
| 3GPP-TVP | 1 |
| ARRI | 1 |
| Convergent | 1 |
| Leica | 1 |
| ISO-MAF | 1 |
| IEC 62592 | 1 |
| Ross | 1 |
| CMAF-ID3 | 1 |
| Blinkbox | 1 |
| Casio | 1 |
| DTS-HD | 1 |
| DxO | 1 |
| MPEG-SD | 1 |
| ISMACryp2 | 1 |
| JPM | 1 |
| JPSearch | 1 |
| JPX | 1 |
| JPXR | 1 |
| MMT | 1 |
| Apple HEVC Alpha | 1 |
| Nikon | 1 |
| Panasonic | 1 |
| PIFF | 1 |
| Panasonic Video Intercom | 1 |
| QT-TN2162 | 1 |
| Avid | 1 |
| see (1) below | 1 |
| FF Video Codec | 1 |
| Avs2 | 1 |
| CamMotion | 1 |
| GB-T-20090-9 | 1 |
| DRA | 1 |
| ITU G.719 | 1 |
| RealHD | 1 |
| SMPTE | 1 |
| UWA | 1 |
| CineForm | 1 |
| Matrox | 1 |

### Typed but not registered

Codes the crate types that no MP4RA table lists, each explained:

| Kind | Code | Why |
|---|---|---|
| SampleEntry | `NONE` | QuickTime uncompressed sound entry; no MP4RA table registers it |
| SampleEntry | `lav3` | AVS3 library sample entry the crate types; MP4RA registers the avs3 entry only |
| SampleEntry | `sowt` | QuickTime little-endian PCM entry; common in .mov files but no MP4RA table registers it |
| SampleEntryBox | `av2c` | AVS2 configuration box the crate types; no MP4RA table registers it |
| SampleEntryBox | `av3c` | AVS3 configuration box the crate types; no MP4RA table registers it |
| SampleEntryBox | `dOps` | Opus decoder configuration box (RFC 7845); no MP4RA table registers it |
| SampleEntryBox | `ddts` | DTS decoder configuration box; MP4RA registers the DTS sample entries but not this box |
| SampleEntryBox | `la2c` | AVS2 library configuration box the crate types; no MP4RA table registers it |
| SampleEntryBox | `lavc` | AVS3 library configuration box the crate types; no MP4RA table registers it |
| ItemProperty | `a1lx` | AVIF layered-image-indexing property the crate types; no MP4RA table registers it |
| ItemProperty | `a1op` | AVIF operating-point selector property the crate types; no MP4RA table registers it |
| ItemProperty | `cmex` | ISO/IEC 23008-12:2025 camera extrinsic matrix property; not yet in MP4RA |
| ItemProperty | `cmin` | ISO/IEC 23008-12:2025 camera intrinsic matrix property; not yet in MP4RA |
| ItemProperty | `prdi` | ISO/IEC 23008-12:2025 progressive derived item information property; not yet in MP4RA |
| ItemProperty | `sstr` | ISO/IEC 23008-12:2025 single-stream property; not yet in MP4RA |
| Reference | `anot` | ISO/IEC 23008-12:2025 region annotation reference; not yet in MP4RA |
| Reference | `drgn` | ISO/IEC 23008-12:2025 derived-region reference; not yet in MP4RA |
| EntityGroup | `prgr` | progressive rendering entity group; MP4RA registers the slideshow and pyramid groups but not this one |
| SampleGroup | `regm` | ISO/IEC 23008-12:2025 region annotation sample group; not yet in MP4RA |

Outside every MP4RA table sit the de-facto QuickTime/iTunes atoms the metadata
layer types: `ilst`, `data`, `mean`, `name`, `----`, `trkn`, `disk`, `covr`,
`chpl`, `gmhd`, `gmin`, `tapt`, `clef`, `prof`, `enof`, `chan` and every
`©`-prefixed international-text atom (the parser types the whole `©` family
generically). MP4RA registers none of them, and the `mdir`/`mdta` metadata
handlers are likewise typed without a registration.

OPEN (§9.2.4.5): the `FDitemconstructorLarge` (constructor type 5) syntax sums
to 20 bytes (1 + 4 + 4 + 8 + 3) while the same clause states every constructor
is 16 bytes, so the two statements cannot both hold. That one constructor is
left untyped and an FD packet carrying it is a typed parse error, rather than
inventing a field layout the clause does not fix.

## Example

Open a file and iterate its samples in five lines:

```rust,no_run
use std::fs::File;
use isobmff::driver::{DriverStep, FileReader};
use isobmff::ReaderConfig;

let mut reader = FileReader::new(File::open("movie.mp4")?, ReaderConfig::default())?;
loop {
    match reader.step()? {
        DriverStep::Sample(sample) => println!("track {} at {}", sample.track, sample.decode_time),
        DriverStep::Done => break,
        _ => {}
    }
}
```

A box builder still works the same way:

```rust,ignore
use broadcast_core::Serialize;
use isobmff::segments::FileTypeBox;

let ftyp = FileTypeBox {
    major_brand: *b"iso5",
    minor_version: 0,
    compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
};
let bytes = ftyp.try_to_bytes().unwrap();
```

Thirteen runnable examples ship with the crate:

```bash
cargo run -p isobmff --example parse_boxes -- fixtures/mp4/h264-aac-prog.mp4
cargo run -p isobmff --example build_ftyp
cargo run -p isobmff --example parse_emsg
cargo run -p isobmff --example build_emsg
cargo run -p isobmff --example list_samples -- fixtures/mp4/progressive/av-prog.mp4
cargo run -p isobmff --example print_audio_tracks -- fixtures/mp4/boxes/track-description.mp4
cargo run -p isobmff --example read_tags -- fixtures/mp4/progressive/av-prog.mp4
cargo run -p isobmff --example write_tags -- fixtures/mp4/progressive/av-prog.mp4 /tmp/tags.udta
cargo run -p isobmff --example heif_info -- fixtures/heif/thumb.heic
cargo run -p isobmff --example build_avif -- fixtures/avif/plain.avif
cargo run -p isobmff --example list_metadata_tracks -- fixtures/mp4/boxes/hint-hmhd.mp4
cargo run -p isobmff --example heif_2025 -- fixtures/heif/groups.mp4
cargo run -p isobmff --example print_hint_tracks -- fixtures/mp4/boxes/hint-offsets.mp4
```

## Still images (HEIF and AVIF)

A still image file has no movie: a file-level `meta` box lists *items*, each
with a type, properties and data addressed by byte ranges. `item_reader` reads
that model without holding the file, and `item_writer` builds one.

What this crate reads (and re-emits byte-exactly): the ISO/IEC 14496-12:2026
§8.11 item boxes, and the image item layer of **ISO/IEC 23008-12:2025** (the
third edition) with **Amendment 1** applied. That is every clause 6.5 property
(including image scaling, the capture-information set, required reference
types, creation/modification time, user description, accessibility text,
sub-sample information, target output layer set, the four transition effects
and their timings, progressive derived image information, single stream, the
camera matrices, and Amendment 1's constrained extents grid, disparity
adjustment, stereo aggressors, reference viewing environment and nominal
diffuse white); the `grid`/`iovl`/`iden` derived bodies plus Amendment 1's
`tmap` tone map and `cfen` colour format enhancement; the clause 6.8 entity
groups (alternates, stereo pair and its monoscopic fallback, bursts, the five
bracketing sets, user collections, panorama, slideshow, progressive rendering,
image pyramid, region partition) with typed `pymd`/`rgpa` bodies; clause 11
regions, mask items and region-track sample groups; clause 8.4 integrity
boxes; clause 10 brands as a typed view with structurally checkable findings;
and clause 7 image-sequence boxes (`ccst`, `auxi`, `refs`). For AVIF (v1.2.0)
it reads the `av01` item type, its `av1C` configuration, the `a1op`/`a1lx`
properties and the AVIF brands. Every property whose syntax another clause
already defines reuses that type; there is no second parser.

What it does not do: decode pixels. Coded item data is opaque and is reported
as ordered byte ranges the caller fetches. The ISO 21496-1 gain-map metadata of
a `tmap` item is typed on the layout open implementations agree on; a body
outside that view stays opaque and round-trips. The ISO/IEC 23090-7 rotation
struct of a version-1 camera extrinsic matrix is carried opaque in a typed
wrapper and never interpreted from memory. The masked regions' pixel data and
the constrained-extents grid's codec-specific per-extent configuration are
likewise opaque. Clause 10's mandatory-box checks, the clause 7.2.1 matrix
constraints and the clause 7.3 hidden-sample rule are listed as OPEN in the
clause-by-clause coverage table kept in the private repo at
`private/specs-verified/mpeg/heif/isobmff-coverage-2025.md`, which also carries
the 2017-vs-2025 re-check.

The item model carries only facts read from the file: brands, the primary
item, each item's type/name/hidden flag/protection index, its properties in
association order with the essential flag, its references in both directions,
its resolved data ranges, the typed region/integrity bodies, and the entity
groups. Image size, rotation, mirroring, thumbnails, auxiliary images, grid
tiles/canvas, overlay layers, the Exif/XMP item ranges, a tone map's
base/gain-map pair, a stereo pair, the region items of an image, a group's
entities and an item's capture information are conveniences derived from those
facts. `StillImage::findings` and `StillImage::structure_findings` report
structurally checkable requirements as typed findings; they never refuse a
read. `StillImage::reemit_plan` re-emits a parsed file through the preserved
structure (exact top-level headers, the `meta` body rebuilt from its typed
children in wire order, and a copy for every payload the reader never read), so
a parsed fixture round-trips byte for byte when the caller supplies its bytes.

Hostile input is capped through `Limits` (item, property and reference counts,
the `meta` box and derived-item bodies; the count caps default to unlimited and
are the caller's to set), a derived-item reference cycle is a typed error, an
extent past the file is a typed error, and an `ipma` index past its `ipco` is a
typed error.

### Still-image evidence

| Structure | Clause | Evidence | File |
|---|---|---|---|
| `udes`, `clli`, `irot`, `imir` | 6.5.20, 6.5.14, 6.5.10, 6.5.12 | real: `heif-enc` 1.23.1 | `heif/props.heic` |
| `grid` derived item | 6.6.2.3 | real: `heif-enc --cut-tiles` | `heif/tiled.heic` |
| `prem` reference, `auxC` | 10.2.3, 6.5.8 | real: `heif-enc --premultiplied-alpha` | `heif/premult.heic` |
| `ster`, `pymd`, `slid`, `prgr` groups | 6.8.5, 6.8.12, 6.8.9, 6.8.10 | real: `MP4Box` entity groups | `heif/groups.mp4` |
| `iscl`, `rref`, `crtt`, `mdft`, `altt` | 6.5.13, 6.5.17-6.5.21 | clause-assembled | — |
| capture properties (`aebr` ... `pano`) | 6.5.22-6.5.27 | clause-assembled | — |
| `tols`, transitions, `stpe`, `ssld`, `prdi`, `sstr` | 6.5.29-6.5.38 | clause-assembled | — |
| `cmex`, `cmin` | 6.5.39, 6.5.40 | clause-assembled (v1 orientation opaque) | — |
| `cexg`, `dadj`, `stag`, `reve`, `ndwt`, `mskC` | Amd 1 6.5.41-45, 11.2.2 | clause-assembled | — |
| `tmap` | Amd 1 6.6.2.4 | real: `avifgainmaputil` 1.4.2 | `avif/gainmap-*.avif` |
| `cfen` | Amd 1 6.6.2.5 | clause-assembled | — |
| `rgan` item, `mski`, region geometry | 11.2-11.3 | clause-assembled | — |
| `rgan` sample entry | 11.4.2.2.2 | clause-assembled (typed in the `stsd` dispatch) | — |
| `eqiv`, `aebr`, `wbbr`, `fobr`, `afbr`, `dobr`, `pano`, `refs`, `regm`, `udes` | 6.8.1.2, 6.8.6, 6.8.8.2, 7.4.1, 11.4.3 | clause-assembled (dispatched from `sgpd`) | — |
| `pymd` body, `rgpa` body | Amd 1 6.8.12-13 | clause-assembled | — |
| `ccst`, `auxi`, `refs`, `md5i` | 7.2.3, 7.5.3, 7.4.1, 8.4 | clause-assembled | — |

Every fixture's item payload range and its FNV-1a 64 hash are pinned in
`tests/it/item_layer_2025.rs`, so a reader that resolves an `idat` extent to the
wrong offset cannot pass.

## Drivers

`Reader` is sans-IO: it asks for structure byte ranges and yields descriptors.
`isobmff::driver` answers those demands for the two blocking source shapes.

- `FileReader<R: Read + Seek>` seeks to each demand. A `moov` after a
  multi-gigabyte `mdat` costs only its headers, and `read_sample` reads one
  sample's bytes on demand.
- `ForwardReader<R: Read>` never seeks: a demand ahead of the current position
  is reached by reading and discarding through one fixed buffer, and a header
  the reader must re-read with a `largesize`/`uuid` extension is answered from
  a retained header tail. A `moov` after an `mdat` is
  `Error::MoovAfterMdatForwardOnly`. It forces `SampleOrder::File`, so a pipe
  always reads samples in ascending file order, and the reader emits a
  fragment's samples before it asks for anything past that fragment's `mdat`:
  `read_sample` therefore works on a fragmented file read from a pipe or
  socket.

Each driver yields `DriverStep::Metadata` once, after the track table, so a
driver user sees the tags as well as the tracks.

Neither driver holds a whole file: each keeps one `moov`/`moof` buffer bounded
by the reader's own `Limits`. For async sources see the `isobmff-tokio` crate;
for HTTP range requests see `broadcast-client`'s `mp4_range` module.

## Extending: private boxes and private tracks

A private box or a private sample entry needs no change to this crate.
Implement `Parse` + `Serialize` as any box here does, then `BoxDef` (its
four-CC or `uuid` extended type, and an optional parent scope), and register it:

```rust,ignore
use isobmff::registry::{BoxDef, BoxRegistry};

impl BoxDef for MyAtom {
    const FOURCC: [u8; 4] = *b"acme";
    const PARENT: Option<[u8; 4]> = Some(*b"udta"); // None: any parent
}

let mut registry = BoxRegistry::new();
registry.register::<MyAtom>()?;             // over a four-CC this crate types
registry.register_override::<MyAtom>()?;    // is an error without `override`
let mut reader = Reader::new(ReaderConfig::default().with_registry(registry));
```

- A registered `stsd` entry comes back typed in `SampleEntryInfo::registered`
  (`RegisteredEntry::downcast_ref::<MyEntry>()`); unregistered, it stays the raw
  entry bytes.
- `BoxRegistry::parse_children(parent, payload)` types the children of a box
  this crate leaves opaque (vendor `udta` atoms, a top-level `uuid`).
- The writer needs no registry: `SampleEntry::from_registered` turns the value
  into the raw entry bytes it carries, and `RegisteredEntry::try_to_bytes` gives
  the box bytes for an opaque child. An unregistered box is an `OpaqueBox` and
  writes back unchanged.

Payload formats (the meaning of the bytes) belong to the plug-in crate, not
here: this crate knows box syntax only. The `isobmff-private-format` workspace
example is a complete, copyable implementation of one private atom and one
private track.

## Round trip

Reading a file and writing it back through `Writer` reproduces it **byte for
byte** for every file this crate can model, because a plan read from a file
carries what the sample tables do not: the parsed `trak` shells (`tkhd` times
and flags, `hdlr` fields, `gmhd`, `tref`), the `moov`'s extra children
(`udta`/`meta`, `iods`, `mvex`), each `stbl` child the plan does not rebuild
(`sgpd`/`sbgp`/`sdtp`) in its wire position, and the top-level boxes around the
movie (`free`, `wide`, `uuid`, `skip`). A plan a `PlanBuilder` made carries
none of it and writes this crate's own conventional layout.

The `isobmff/tests/it/whole_file_round_trip.rs` test asserts the invariant over
every committed progressive fixture, the tagged `.m4a` set, the GoPro captures
and the QuickTime `.mov` set. Files that cannot be byte-identical are listed
there with the exact reason and compared structurally instead: the fragmented
fixtures state their sample layout differently in `moof` (a `tfhd` with sample
defaults or `base_data_offset`, a `trun` with a per-run keyframe flag) and end
with a trailing `mfra`, and one carries a top-level `sidx`; the writer emits
per-sample `trun` fields under `default-base-is-moof`, no `mfra` and no `sidx`.

## Coming from `mp4` or `mp4parse`

Those crates expose a whole-item parse: read a header, then read samples by
index from the same in-memory or seekable source. This crate keeps the same
two jobs but splits them into a sans-IO state machine and a driver:

- open the file once with `FileReader::new` (or `ForwardReader::new` for a
  source that cannot seek);
- call `step` until it returns `DriverStep::Done`, handling
  `DriverStep::Tracks`, `DriverStep::Fragment` and `DriverStep::Sample`;
- call `read_sample` when you want one sample's bytes.

The track table and the sample descriptors are plain data, so a consumer that
only needs an index never reads media.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | yes | `std` glue; without it the crate is `no_std` + `alloc` |
| `serde` | no | `serde::Serialize` on the box types |
| `scan-counters` | no | test instrumentation: `Reader::bytes_requested()` counts the bytes a run asked for |

## Spec citations

- **Box / FullBox**: ISO/IEC 14496-12:2026 §4.2.
- **Skip-unknown rule**: §4.2 — unknown type → ignore + skip via `size`.
- **CENC boxes**: ISO/IEC 23001-7 §12.
- **`emsg`**: ISO/IEC 23009-1 §5.10.3.3; DASH-IF IOP Part 10 §6.1.
- **Metadata (`udta`/`meta`/`keys`/`ilst`)**: QuickTime File Format
  (`private/specs-verified/apple/isobmff/quicktime-file-format-online.md`),
  ISO/IEC 14496-12:2026 §8.10.1/§8.11, and 3GPP TS 26.244 Table 8.10 for
  `loci`.

## Tier 3 and tier 4 box evidence

The tier 3 (metadata tracks and file-level extras) and tier 4 (newer
signalling) boxes of issue #4167 follow the same rule as tier 1: a fixture the
crate has, written by another tool, is asserted field by field against that
tool's dump and re-serialized byte-exact; a box no installed tool writes says
so and is tested from the clause. `fixtures/mp4/boxes/PROVENANCE.md` records
every command, tool version and dump.

Every committed MP4-family file (245 under `fixtures/` and `private/fixtures/`)
was walked with `BoxIter`, descending sample entries. The tier 3/4 boxes they
carry are `mehd` (19 DASH init segments), `nmhd` (7 subtitle fixtures), `hmhd`
(`hint-hmhd.mp4`), `cprt` (`cprt.mp4`), and `keyd` plus its Apple setup boxes
in the private iPhone Live Photo's two `mebx` entries.

| Box | Clause | Evidence | File |
|---|---|---|---|
| `cprt` | §8.10.2 | real: written by GPAC 26.07 `-cprt`, dumped by `-disox` | `mp4/boxes/cprt.mp4` |
| `hmhd` | §12.4.3.1 | real: written by GPAC 26.07 `-hint`, dumped by `-disox` | `mp4/boxes/hint-hmhd.mp4` |
| `nmhd` | §8.4.5.2 | real: 7 committed subtitle fixtures | `subtitles/mp4box/wvtt_progressive.mp4`, … |
| `mehd` | §8.8.2 | real: 19 committed DASH init segments | `dash/ondemand-gpac-pto-quirk/v_dashinit.mp4`, … |
| `keyd` | §12.8.4.4 | real: Apple's Live Photo `mebx` key table | private `mp4/quicktime/iphone-x-live-photo.mov` |
| `uri `, `uriI` | §12.3.3 | clause-assembled | — |
| `metx`, `mett`, `stxt`, `txtC`, `mime` | §12.3.3, §12.5.3 | clause-assembled | — |
| `urim`, `mebx` | §12.3.3, §12.8.4.1 | `mebx` real in the Live Photo's bytes; the rest clause-assembled | private `mp4/quicktime/iphone-x-live-photo.mov` |
| `xml `, `bxml` | §8.11.2 | clause-assembled | — |
| `loca`, `setu` | §12.8.4.5, §12.8.4.6 | clause-assembled | — |
| `etyp`, `tyco`, `otyp` | §5.3.2, §8.16.5 | clause-assembled | — |
| `imda` | §8.1.4 | clause-assembled | — |
| `strk`, `stri`, `strd` | §8.13.3–5 | clause-assembled | — |
| `vvhd` | §12.9.2 | clause-assembled | — |
| `rinf`, `csch` | §13.5.3, §13.5.5 | clause-assembled | — |
| `cinf` | §13.6.3 | clause-assembled | — |
| `scrb`, `spki` | §13.4.8, §13.5.6.2 | clause-assembled | — |
| `amve`, `cclv` | §12.1.9, §12.1.8 | clause-assembled | — |
| `ardi`, `aelm`, `aedb`, `aepp`, `aepr`, `aesb`, `aesd` | §12.2.8–14 | clause-assembled | — |
| `assp` | §8.8.16 | clause-assembled | — |

What was tried per family, and why no tool produced the rest:

| Attempt | Result |
|---|---|
| `MP4Box -add fixtures/mp4/h264-aac-prog.mp4 -cprt "(c) 2026 Example Corp" -new cprt.mp4` | wrote the `cprt` fixture |
| `MP4Box -hint -add fixtures/mp4/h264-aac-prog.mp4 -new hint-hmhd.mp4` | wrote two hint tracks, each with an `hmhd` |
| `MP4Box -add h264-aac-prog.mp4 -add meta.xml -new out.mp4`, `-add meta.txt` | exit 1, no metadata track |
| `gpac -i meta.xml -o out.mp4`, `gpac -i meta.txt -o out.mp4` | "Filter not found for the desired type" |
| `MP4Box -brand iso6 -ab iso5 -add h264-aac-prog.mp4 -new out.mp4` | rewrote `ftyp` only; no `etyp` |
| `MP4Box -dref -add h264-aac-prog.mp4 -new out.mp4` | exit 1; no `imda`, and `-dref` writes an in-file `url ` entry |
| `MP4Box -add h264-aac-prog.mp4 -zmov -new out.mp4` | a compressed `!mov` box, no `otyp` |
| `ffmpeg -c copy -movflags frag_keyframe+empty_moov` | `mvex` but no `mehd` |
| `ffmpeg -i iphone-x-live-photo.mov -map 0:2 -c copy out.mp4` | "Could not find tag for codec none"; no metadata track written |
| `avconvert --preset PresetHEVCHighestQuality` on the Live Photo | kept the two `mebx` entries (no new box), so the private file stays the `keyd` fixture |

The committed GPAC DASH init segments carry `mehd`, which is why that box
needs no new file. The audio element, colour volume, sub-track, scheme and
alternative-startup boxes have no writer in GPAC 26.07 or ffmpeg 8.1.2 at
all.

### Registered-code evidence

| Code | Clause | Evidence | File |
|---|---|---|---|
| `sthd` | ISO/IEC 14496-12:2026 §12.6.2 | real: `MP4Box -diso` (Size 12, Version 0, Flags 0) | `mp4/stpp.mp4` |
| `wide` | QuickTime File Format, Wide atom | real: `MP4Box -diso` (Size 8) | `audio/srat/qt-alac-v1.mov` |
| `c608` | QuickTime File Format, closed captioning sample description | real: `MP4Box -diso` (Size 16) | private `candidates/shaka/cea608-track-init.mp4` |
| `tran` | ISO/IEC 14496-15:2010 C.2.10 | clause-assembled: no fixture carries it and no installed tool writes it | — |
| `Brand`, `HandlerType` | the tables above | real: `MP4Box -diso` (`isom`, `subt`, `qt  `, `soun`) | `mp4/stpp.mp4`, `audio/srat/qt-alac-v1.mov` |
| `SampleGroupType` | ISO/IEC 14496-12:2026 §10 | real: `MP4Box -diso` (`grouping_type="roll"`, sample_count 87) | `dash/multiperiod-restart/p1_dash_track2_1.m4s` |
| `TrackReferenceType` | ISO/IEC 14496-12:2026 §8.3.3 | real: `MP4Box -diso` (`hint`, TrackID 1/2) | `mp4/boxes/hint-hmhd.mp4` |
