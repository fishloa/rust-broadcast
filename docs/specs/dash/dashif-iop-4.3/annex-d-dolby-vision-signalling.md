---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "Annex D Signaling Dolby Vision Profiles and Levels (D.1 and printed B.1.x sub-clauses)"
pdf_pages: "241-244"
printed_pages: "annex pp.13-16"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/annex-d-dolby-vision-signalling.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 Annex D - Dolby Vision profile and level strings (notes)

Own-words notes with the facts a client needs to parse `@codecs`; exact text in the private file. "p." = PDF page.

## Profiles (Table D.1, p.241)

| Profile ID | Name | BL / EL codec | BL:EL | BL backward compat | Full BL/EL alignment | Profile |
|---|---|---|---|---|---|---|
| 2 | `dvhe.der` | HEVC8 / HEVC8 | 1:1/4 | SDR | no | Main / Main |
| 3 | `dvhe.den` | HEVC8 / HEVC8 | 1:1 | none | no | Main / Main |
| 4 | `dvhe.dtr` | HEVC10 / HEVC10 | 1:1/4 | SDR | no | Main10 / Main10 |
| 5 | `dvhe.stn` | HEVC10 / n.a. | n.a. | none | n.a. | Main10 / n.a. |
| 6 | `dvhe.dth` | HEVC10 / HEVC10 | 1:1/4 | HDR10 | no | Main10 / Main10 |
| 7 | `dvhe.dtb` | HEVC10 / HEVC10 | 1:1/4 UHD, 1:1 FHD | Blu-ray HDR | no | Main10 / Main10 |

Full alignment means EL GOP/sub-GOP structure and POC match the BL so seeking works anywhere. Encoder advice: use the baseline composer for non-backward-compatible profiles; dual-layer encoders should align GOP/sub-GOP structure for all listed profiles (the text points at "Table 4", i.e. D.1).

## Profile string grammar (D.1.1.1, p.242)

Form: `dv[BL codec].[layers][bit depth][compat][EL codec][EL bit depth]`.

| Field | Letters |
|---|---|
| `dv` | Dolby Vision |
| BL codec | `he` = HEVC |
| layers | `s` single; `d` dual without GOP/POC alignment enforcement; `p` dual with enforcement |
| bit depth | `e` = 8, `t` = 10 |
| compat | `n` none; `r` SDR (Rec.709, 100 nits); `h` HDR10; `b` Blu-ray HDR |
| EL codec | `a` AVC, `h` HEVC |
| EL bit depth | `e` 8, `t` 10 |

EL codec and depth appear only when the EL codec differs from the BL codec. No interlaced support; codecs other than HEVC/AVC may come later.

## Level table and string (Table B.2 and B.1.2.1, p.243)

| ID | Name | Example max | Max Mbps main / high tier |
|---|---|---|---|
| 1 | hd24 | 1280x720x24 | 20 / 50 |
| 2 | hd30 | 1280x720x30 | 20 / 50 |
| 3 | fhd24 | 1920x1080x24 | 20 / 70 |
| 4 | fhd30 | 1920x1080x30 | 20 / 70 |
| 5 | fhd60 | 1920x1080x60 | 20 / 70 |
| 6 | uhd24 | 3840x2160x24 | 25 / 130 |
| 7 | uhd30 | 3840x2160x30 | 25 / 130 |
| 8 | uhd48 | 3840x2160x48 | 40 / 130 |
| 9 | uhd60 | 3840x2160x60 | 40 / 130 |

Rates are for BL and EL combined; level fixes maximum pixel rate so lower resolution can trade for higher frame rate. Level string: `[resolution][fps][h]` with `hd`=720, `fhd`=1080, `uhd`=2160, fps as number, trailing `h` when the high tier / higher frame rates apply (printed sub-clause numbers B.1.2.1, B.1.3, B.1.3.1 sit inside Annex D).

## Combined string and device capabilities (B.1.3-B.1.3.1, pp.243-244)

Recommended codec string = `[profile string].[level string]`; examples `dvav.per.fhd30` (dual-layer AVC 8-bit, enforced alignment, Rec.709-compatible, 1080p30) and `dvhe.stn.uhd30` (single-layer HEVC 10-bit, non-compatible, 2160p30). A device may keep an array of supported strings and match against the manifest's stream types, or advertise them in the HTTP User-Agent as `+`-separated `dv...` entries appended to the UA string (a server searches for `+dv`).
