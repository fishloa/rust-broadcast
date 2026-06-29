# Box order rules — ISO/IEC 14496-12:2015 §6.2

Transcribes the box-ordering constraints and encapsulation hierarchy from ISO 14496-12:2015
§6.2 (Metadata Structure / Box Order) that a muxer must follow. Source: the verified PDF
transcription object.

---

## §6.2.3 Box Order — numbered ordering constraints

Source: ISO/IEC 14496-12:2015 §6.2.3 (clause "Box Order")

The following rules and guidelines shall be followed for the order of boxes within an
ISO Base Media File. Rules use "shall" (normative); recommendations use "should" or
"recommended".

1. **`ftyp` first before variable-length boxes.** The File Type Box (`ftyp`) shall
   occur before any variable-length box (e.g. Movie Box, Free Space Box, Media Data
   Box). Only a fixed-size box such as a file signature (if required) may precede it.

2. **Header boxes first in their container.** It is **strongly recommended** that all
   header boxes be placed first in their container: Movie Header Box (`mvhd`),
   Track Header Box (`tkhd`), Media Header Box (`mdhd`), and the specific media
   headers inside the Media Information Box (e.g. Video Media Header `vmhd`).

3. **Movie Fragments in sequence order.** Any Movie Fragment Boxes (`moof`) shall
   be in sequence order per §8.8.5 (the `mfhd.sequence_number` must increase
   monotonically).

4. **Sample Table Box order.** It is recommended that the boxes within the Sample
   Table Box (`stbl`) be in the following order:
   - Sample Description Box (`stsd`)
   - Time to Sample Box (`stts`)
   - Sample to Chunk Box (`stsc`)
   - Sample Size Box (`stsz`)
   - Chunk Offset Box (`stco`)

5. **Track Reference and Edit List before Media Box.** It is **strongly recommended**
   that the Track Reference Box (`tref`) and Edit List (`edts`) precede the Media
   Box (`mdia`); the Handler Reference Box (`hdlr`) should precede the Media
   Information Box (`minf`); and the Data Information Box (`dinf`) should precede
   the Sample Table Box (`stbl`).

6. **User Data Boxes last.** It is recommended that User Data Boxes (`udta`) be
   placed last in their container (either the Movie Box `moov` or Track Box `trak`).

7. **`mfra` last in file.** It is recommended that the Movie Fragment Random Access
   Box (`mfra`), if present, be last in the file.

8. **Progressive download info box early.** It is recommended that the Progressive
   Download Information Box (`pdin`) be placed as early as possible in files, for
   maximum utility.

---

## §6.2.3 Table 1 — Container / Quantity / Mandatory summary

Source: ISO/IEC 14496-12:2015 §6.2.3 Table 1 (informative box-type cross-reference)

The following table reproduces the containment hierarchy from Table 1. An asterisk
(`*`) marks mandatory boxes.

### Top-level boxes

| Box      | Mandatory | §     | Description                            |
|----------|-----------|-------|----------------------------------------|
| `ftyp`   | *         | 4.3   | File type and compatibility            |
| `pdin`   |           | 8.1.3 | Progressive download information       |
| `moov`   | *         | 8.2.1 | Container for all metadata             |
| `mfra`   |           | 8.8.9 | Movie fragment random access           |
| `moof`   |           | 8.8.4 | Movie fragment                         |
| `mdat`   |           | 8.2.2 | Media data                             |
| `free`   |           | 8.1.2 | Free space                             |
| `skip`   |           | 8.1.2 | Free space                             |
| `styp`   |           | 8.16.2| Segment type                           |
| `sidx`   |           | 8.16.3| Segment index                          |
| `ssix`   |           | 8.16.4| Subsegment index                       |
| `prft`   |           | 8.16.5| Producer reference time                |
| `meta`   |           | 8.11.1| Metadata                               |

### `moov` → children

| Box      | Mandatory | §     | Description                            |
|----------|-----------|-------|----------------------------------------|
| `mvhd`   | *         | 8.2.2 | Movie header, overall declarations      |
| `trak`   | *         | 8.3.1 | Container for a track/stream            |
| `mvex`   |           | 8.8.1 | Movie extends (warns of fragments)      |
| `meta`   |           | 8.11.1| Metadata                                |
| `udta`   |           | 8.10.1| User data                               |

### `trak` → children

| Box      | Mandatory | §     | Description                            |
|----------|-----------|-------|----------------------------------------|
| `tkhd`   | *         | 8.3.2 | Track header                           |
| `tref`   |           | 8.3.3 | Track reference container              |
| `trgr`   |           | 8.3.4 | Track grouping indication              |
| `edts`   |           | 8.6.5 | Edit list container                    |
| `meta`   |           | 8.11.1| Metadata                                |
| `mdia`   |           | 8.4   | Container for media information        |
| `udta`   |           | 8.10.1| User data                               |

### `mdia` → children

| Box      | Mandatory | §     | Description                            |
|----------|-----------|-------|----------------------------------------|
| `mdhd`   | *         | 8.4.2 | Media header                           |
| `hdlr`   | *         | 8.4.3 | Handler reference                      |
| `elng`   |           | 8.4.6 | Extended language tag                  |
| `minf`   | *         | 8.4.4 | Media information container            |

### `minf` → children (per handler type)

| Box      | Mandatory    | §      | Description                            |
|----------|--------------|--------|----------------------------------------|
| `vmhd`   | * (video)    | 12.1.2 | Video media header                     |
| `smhd`   | * (sound)    | 12.2.2 | Sound media header                     |
| `hmhd`   | * (hint)     | 12.4.2 | Hint media header                      |
| `sthd`   | * (subtitle) | 12.6.2 | Subtitle media header                  |
| `nmhd`   | * (other)    | 8.4.5.2| Null media header                      |
| `dinf`   | *            | 8.7.1  | Data information (container)           |
| `stbl`   | *            | 8.5.1  | Sample table (container)               |

### `dinf` → children

| Box      | Mandatory | §     | Description                            |
|----------|-----------|-------|----------------------------------------|
| `dref`   | *         | 8.7.2 | Data reference box                     |

### `stbl` → children

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `stsd`   | *         | 8.5.2  | Sample description                     |
| `stts`   | *         | 8.6.1.2| Decoding time-to-sample                |
| `ctts`   |           | 8.6.1.3| Composition time-to-sample             |
| `cslg`   |           | 8.6.1.4| Composition to decode timeline         |
| `stsc`   | *         | 8.7.4  | Sample-to-chunk                        |
| `stsz`   | * (or `stz2`) | 8.7.3.2| Sample sizes                         |
| `stz2`   | * (or `stsz`) | 8.7.3.3| Compact sample sizes                 |
| `stco`   | * (or `co64`) | 8.7.5  | Chunk offset (32-bit)                |
| `co64`   | * (or `stco`) | 8.7.5  | Chunk offset (64-bit)                |
| `stss`   |           | 8.6.2  | Sync sample                            |
| `stsh`   |           | 8.6.3  | Shadow sync sample                     |
| `padb`   |           | 8.7.6  | Sample padding bits                    |
| `stdp`   |           | 8.7.6  | Sample degradation priority            |
| `sdtp`   |           | 8.6.4  | Independent and disposable samples     |
| `sbgp`   |           | 8.9.2  | Sample-to-group                        |
| `sgpd`   |           | 8.9.3  | Sample group description               |
| `subs`   |           | 8.7.7  | Sub-sample information                 |
| `saiz`   |           | 8.7.8  | Sample auxiliary info sizes            |
| `saio`   |           | 8.7.9  | Sample auxiliary info offsets          |
| `udta`   |           | 8.10.1 | User data                              |

### `mvex` → children

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `mehd`   |           | 8.8.2  | Movie extends header                   |
| `trex`   | *         | 8.8.3  | Track extends defaults                 |
| `leva`   |           | 8.8.13 | Level assignment                       |
| `trep`   |           | 8.8.15 | Track extension properties             |

### `moof` → children

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `mfhd`   | *         | 8.8.5  | Movie fragment header                  |
| `traf`   |           | 8.8.6  | Track fragment                         |
| `meta`   |           | 8.11.1 | Metadata                               |

### `traf` → children

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `tfhd`   | *         | 8.8.7  | Track fragment header                  |
| `trun`   |           | 8.8.8  | Track fragment run                     |
| `sbgp`   |           | 8.9.2  | Sample-to-group                        |
| `sgpd`   |           | 8.9.3  | Sample group description               |
| `subs`   |           | 8.7.7  | Sub-sample information                 |
| `saiz`   |           | 8.7.8  | Sample auxiliary info sizes            |
| `saio`   |           | 8.7.9  | Sample auxiliary info offsets          |
| `tfdt`   |           | 8.8.12 | Track fragment decode time             |
| `meta`   |           | 8.11.1 | Metadata                               |

### `mfra` → children

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `tfra`   |           | 8.8.10 | Track fragment random access           |
| `mfro`   | *         | 8.8.11 | Movie fragment random access offset    |

### `sinf` → children (inside protected sample entries)

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `frma`   | *         | 8.12.2 | Original format                        |
| `schm`   |           | 8.12.5 | Scheme type                            |
| `schi`   |           | 8.12.6 | Scheme information                     |

### `rinf` → children (inside restricted sample entries)

| Box      | Mandatory | §      | Description                            |
|----------|-----------|--------|----------------------------------------|
| `frma`   | *         | 8.12.2 | Original format                        |
| `schm`   | *         | 8.12.5 | Scheme type                            |
| `schi`   |           | 8.12.6 | Scheme information                     |
