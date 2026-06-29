# Local media-spec library (gitignored PDFs)

ISO/IEC + codec/container spec PDFs vendored **locally for transcription reference, never committed** (copyright; caught by `.gitignore` `specs/iso_iec_*.pdf`). Source: the SRS/ossrs free spec collection — index https://ossrs.net/lts/en-us/docs/v5/tools/specs (files at `ossrs.io|ossrs.net/lts/zh-cn/assets/files/`). See memory `ossrs-spec-collection.md`.

**Re-fetch:** ossrs serves with a broken TLS cert → `curl -k` (and run with the sandbox disabled if the host is blocked).

| Local filename (`specs/`) | Spec | Text layer? | Covers |
|---|---|---|---|
| `iso_iec_14496-1_systems_es_descriptor_2010.pdf` | ISO/IEC 14496-1:2010 Systems | **TEXT (verifiable)** | ES_Descriptor / DecoderConfigDescriptor / DecoderSpecificInfo (full `esds` internals) |
| `iso_iec_14496-3_aac_audiospecificconfig_2001.pdf` | ISO/IEC 14496-3:2001 AAC | **TEXT** | AudioSpecificConfig (AAC, inside esds) |
| `iso_iec_14496-10_avc_h264_2003.pdf` | ISO/IEC 14496-10 / ITU-T H.264 AVC | **TEXT** | H.264 NAL / SPS / PPS syntax (free copy of the ITU spec) |
| `iso_iec_14496-12_isobmff_2015.pdf` | ISO/IEC 14496-12:2015 ISOBMFF | **TEXT** | base box format (ftyp/moov/moof/trun/sidx/edts/saiz/sgpd/sinf/…); used for the transmux box docs |
| `iso_iec_14496-14_mp4_2003.pdf` | ISO/IEC 14496-14:2003 MP4 | **TEXT** | MP4 file format, `esds` box (ESDBox) |
| `iso_iec_14496-15_nal_avc_hevc_2017_SCANNED.pdf` | ISO/IEC 14496-15:2017 NAL file format | **SCANNED (image, OCR-only — NOT value-verifiable)** | avcC / hvcC decoder config records — cross-check vs FFmpeg only |
| `iso_iec_23009-1_dash_2012.pdf` | ISO/IEC 23009-1:2012 DASH | **TEXT** | MPD / DASH manifest |

**Verification discipline:** TEXT-layer PDFs are value-verifiable via `pdf2md ... --report` (exit 0). The SCANNED one (14496-15) can only be OCR'd → treat as a cross-reference, never a verified primary (use FFmpeg `movenc.c` as the verifiable source for avcC/hvcC; the scan cross-checks it).

**Still NOT obtained (paid / not in this collection):** ISO/IEC 23001-7 (Common Encryption / CENC — only a watermarked preview found; CENC deferred), a text-layer 14496-15.
