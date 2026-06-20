# MMI — Low-Level / Display / Keypad / Subtitle / Download objects

_Source: EN 50221 §8.6.2-§8.6.4, Tables 34-45 (PDF pp. 36-45), render-verified_

The Man-Machine Interface resource (resource identifier `00400041`, see Table 57)
defines two interaction levels. **Low-Level MMI** mode gives the application detailed
control: the display profile, the keypad/key codes, and the DVB Subtitling display
mechanism (text in low-level mode is coded per reference [9], the DVB Subtitling
specification). This file covers the display-control, keypad, subtitle and download
objects of §8.6.2-§8.6.4. The high-level menu/list objects (Tables 46-51) are in
`mmi-high-level.md`; the Close MMI object (Table 33) is in `mmi-close.md`.

apdu_tag values (cross-ref Table 58, `apdu-tag-values.md`):

| apdu_tag | tag value | Direction (host <-> app) |
|----------|-----------|--------------------------|
| Tdisplay_control        | `9F 88 01` | `<---` |
| Tdisplay_reply          | `9F 88 02` | `--->` |
| Tkeypad_control         | `9F 88 05` | `<---` |
| Tkeypress               | `9F 88 06` | `--->` |
| Tsubtitle_segment_last  | `9F 88 0E` | `<---` |
| Tsubtitle_segment_more  | `9F 88 0F` | `--->` |
| Tdisplay_message        | `9F 88 10` | `<---` |
| Tscene_end_mark         | `9F 88 11` | `<---` |
| Tscene_done             | `9F 88 12` | `<---` |
| Tscene_control          | `9F 88 13` | `--->` |
| Tsubtitle_download_last | `9F 88 14` | `<---` |
| Tsubtitle_download_more | `9F 88 15` | `--->` |
| Tflush_download         | `9F 88 16` | `<---` |
| Tdownload_reply         | `9F 88 17` | `<---` |

> Direction note: the `_last` / `_more` pairs are the APDU-chaining `L_apdu_tag` /
> `M_apdu_tag` variants of the same object body (last block vs. more-to-follow); see
> `apdu-coding.md`. The directions above are reproduced verbatim from Table 58
> (`apdu-tag-values.md`); subtitle_segment_more and subtitle_download_more are listed
> there as `--->` while their `_last` partners are `<---`.

## §8.6.2 Objects used in both modes

### Table 34 — Display Control object coding (display_control)

apdu_tag `Tdisplay_control` = `9F 88 01`, Direction `<---` (EN 50221 §8.6.2.2, Table 34, PDF p. 37).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `display_control() {` | | |
| &nbsp;&nbsp;display_control_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;display_control_cmd | 8 | uimsbf |
| &nbsp;&nbsp;`if (display_control_cmd == set_MMI_mode) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;MMI_mode | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

The display control object both requests information about the display
characteristics and sets the display mode for information display.

#### display_control_cmd values (EN 50221 §8.6.2.2, Table p. 37)

| display_control_cmd | value | description |
|---------------------|-------|-------------|
| set_mmi_mode                          | `01` | Request that host enters the MMI mode indicated by the MMI_mode byte. |
| get_display_character_table_list      | `02` | Request the host return a list of the character code tables it can support during display operations. |
| get_input_character_table_list        | `03` | Request the host return a list of the character code tables it can support during input operations. |
| get_overlay_graphics_characteristics  | `04` | Request the profile of the display when used to display graphics overlaid over video. |
| get_full-screen_graphics_characteristics | `05` | Request the profile of the display when used to display graphics in replacement of video. |
| reserved                              | other values | |

#### mmi_mode values (EN 50221 §8.6.2.2, Table p. 37)

| mmi_mode | value | description |
|----------|-------|-------------|
| high level                | `01` | Request that a high level MMI session is opened. If implemented on the main video display this may partially or completely obscure any video currently being displayed. |
| low level overlay graphics | `02` | Request that a graphical low level MMI session is opened overlaying the main video display (if one is active). |
| low level full screen graphics | `03` | Request that a graphical low level MMI session is opened replacing (or independent of) the main video display. |
| reserved                  | other values | |

### Table 35 — Display Reply object coding (display_reply)

apdu_tag `Tdisplay_reply` = `9F 88 02`, Direction `--->` (EN 50221 §8.6.2.3, Table 35, PDF p. 38).
Response by the host's display system to the display control objects.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `display_reply() {` | | |
| &nbsp;&nbsp;display_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;display_reply_id | 8 | uimsbf |
| &nbsp;&nbsp;`if (display_reply_id == list_graphic_overlay_characteristics \|\|` | | |
| &nbsp;&nbsp;`    display_reply_id == list_full_screen_graphic_characteristics ) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;display_horizontal_size | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;display_vertical_size | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;aspect_ratio_information | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;graphics_relation_to_video | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;multiple_depths | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;display_bytes | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;composition_buffer_bytes | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;object_cache_bytes | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;number_pixel_depths | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0;i<n;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;display_depth | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;pixels_per_byte | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;region_overhead | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (display_reply_id == list_display_character_tables \|\|` | | |
| &nbsp;&nbsp;`    display_reply_id == list_input_character_tables ) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0;i<n;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;character_table_byte | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (display_reply_id == mmi_mode_ack) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;/* acknowledge of the selected mmi mode */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;mmi_mode | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

#### display_reply_id values (EN 50221 §8.6.2.3, Table p. 38)

| display_reply_id | id value |
|------------------|----------|
| mmi_mode_ack                          | `01` |
| list_display_character_tables         | `02` |
| list_input_character_tables           | `03` |
| list_graphic_overlay_characteristics  | `04` |
| list_full_screen_graphic_characteristics | `05` |
| unknown display_control_cmd           | `F0` |
| unknown_mmi_mode                      | `F1` |
| unknown_character_table               | `F2` |
| reserved                              | other values |

Field semantics (EN 50221 §8.6.2.3, p. 39):

- `display_horizontal_size` / `display_vertical_size` — 16-bit integers giving the
  maximum addressable co-ordinate range of the bit-mapped graphic display; positions
  (0,0) to (h_size-1, v_size-1) can be addressed.
- `aspect_ratio_information` — 4-bit field, coded as the ISO/IEC 13818-2
  aspect_ratio_information field; lets the pixel aspect ratio be determined.
- `graphics_relation_to_video` — 3-bit field: `000` = no relationship between
  graphics and video; `001`-`110` = reserved; `111` = the graphics co-ordinate space
  exactly matches the video co-ordinate space.
- `multiple_depths` — 1 = display can mix different pixel depths per region; 0 =
  (unspecified) restrictions mean only a single pixel depth for all regions.
- `display_bytes` — 12-bit integer; ×256 gives the bytes available for display memory.
- `composition_buffer_bytes` — 8-bit integer; ×256 gives the bytes available for the
  composition buffer.
- `object_cache_bytes` — 8-bit integer; ×4096 gives the bytes available for the object
  cache buffer.
- `number_pixel_depths` — 4-bit integer; number of different pixel depths the display
  can provide (the count `n` of the following loop).
- `display_depth` — 3-bit field; the display pixel depth, coded as
  region_level_of_compatibility in [9].
- `pixels_per_byte` — 3-bit integer; pixels packed per byte at this depth; value 0 is
  a special case implying an 8-bit-deep display.
- `region_overhead` — 8-bit integer; ×16 gives the reduction in displayable pixels
  when an additional region is introduced at this pixel depth.
- `character_table_byte` — the character-table selection bytes defined by [4]
  (non-ordered list). All hosts must support the default (table 0) Latin Alphabet.

## §8.6.3 Low-Level MMI Keypad objects (low level mode only)

### Table 36 — Keypad Control object coding (keypad_control)

apdu_tag `Tkeypad_control` = `9F 88 05`, Direction `<---` (EN 50221 §8.6.3.1, Table 36, PDF p. 40).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `keypad_control() {` | | |
| &nbsp;&nbsp;keypad_control_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;keypad_control_cmd | 8 | uimsbf |
| &nbsp;&nbsp;`if (keypad_control_cmd == intercept_selected_keypress) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0;i<keypad_control_length - 1;i++) {`&nbsp;&nbsp;/* list of accepted keypresses */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;key_code | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (keypad_control_cmd == ignore_selected_keypress) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0;i<keypad_control_length - 1;i++) {`&nbsp;&nbsp;/* list of ignored keypresses */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;key_code | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (keypad_control_cmd == reject_keypress) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;key_code&nbsp;&nbsp;/* rejected keypress */ | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

#### keypad_control_cmd values (EN 50221 §8.6.3.1, Table p. 41)

| keypad_control_cmd | cmd value |
|--------------------|-----------|
| intercept_all_keypresses    | `01` |
| ignore_all_keypresses       | `02` |
| intercept_selected_keypress | `03` |
| ignore_selected_keypress    | `04` |
| reject_keypress             | `05` |
| reserved                    | other values |

The Keypad Control object directs virtual keypresses to the application; keypresses
are then delivered via the Keypress object. The application can intercept, ignore or
reject keypresses.

### Table 37 — Keypress object coding (keypress)

apdu_tag `Tkeypress` = `9F 88 06`, Direction `--->` (EN 50221 §8.6.3.2, Table 37, PDF p. 41).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `keypress() {` | | |
| &nbsp;&nbsp;keypress_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field()=1 | | |
| &nbsp;&nbsp;key_code | 8 | uimsbf |
| `}` | | |

#### Table of key codes (EN 50221 §8.6.3.3, Table p. 41)

| key code (hex) | meaning |
|----------------|---------|
| `0`-`9` | digits 0-9 |
| `A`  | menu |
| `B`  | ESC |
| `C`  | ⇒ (right) |
| `D`  | ⇐ (left) |
| `E`  | ⇑ (up) |
| `F`  | ⇓ (down) |
| `10` | BS (backspace) |
| `11` | RC |

Other values (from `0x12` to `0xFF`) are reserved. It is mandatory to support all the
key codes; the corresponding keys are not necessarily present on the keypad.

> Render note: the up/down arrow glyphs in the p. 41 key-code table are double-line
> arrows (⇑/⇓); the left/right are rendered as ⇒/⇐. Transcribed as printed.

## §8.6.4 Low-Level MMI Display objects (DVB Subtitling mechanism)

### Table 38 — subtitle_segment object coding (subtitle_segment)

apdu_tag pair: `Tsubtitle_segment_last` = `9F 88 0E` / `Tsubtitle_segment_more` =
`9F 88 0F` (EN 50221 §8.6.4.2, Table 38, PDF p. 42). The more/last format allows long
segments to be fragmented over multiple APDU.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `subtitle_segment() {` | | |
| &nbsp;&nbsp;subtitle_segment_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;DVB_Subtitling_segment() | | |
| `}` | | |

`DVB_Subtitling_segment()` — a DVB Subtitling segment as defined by reference [9]
(prETS 300 743); see `dvb-subtitle`. Its page ID can be ignored by the decoder and
need not be set by the application (§8.6.4.6, p. 46).

### Table 39 — Display Message object coding (display_message)

apdu_tag `Tdisplay_message` = `9F 88 10`, Direction `<---` (EN 50221 §8.6.4.3, Table 39, PDF p. 42).
Alerts the application to situations that require attention.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `display_message() {` | | |
| &nbsp;&nbsp;display_message_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;display_message_id | 8 | uimsbf |
| `}` | | |

#### display_message_id values (EN 50221 §8.6.4.3, Table p. 42)

| display message_id | value | when used |
|--------------------|-------|-----------|
| Display OK                     | `00` | Can optionally be sent by the host as a positive acknowledgement of any low or high level MMI object. |
| Display Error                  | `01` | An error has been detected by the display system. |
| Display out of memory          | `02` | The host has exhausted its composition buffer, pixel buffer or object cache memory. |
| DVB Subtitling syntax error    | `03` | The host cannot interpret the DVB Subtitling Segments. |
| Undefined region referenced    | `04` | A reference to a region_id that has not been introduced. |
| Undefined CLUT referenced      | `05` | A reference to a CLUT_id that has not been introduced. |
| Undefined object referenced    | `06` | A reference to an object_id that has not been introduced. |
| Object incompatible with region | `07` | The pixel depth or size of an object is not compatible with the region where it is instanced. |
| Unknown character referenced   | `08` | A character code incompatible with the selected character table has been found. |
| Display characteristics changed | `09` | Some characteristic of the display has changed since the display was last inspected by the application (e.g. a 16:9 → 4:3 reconfiguration, or a change in program material / video display format). |
| reserved                       | other values | |

### §8.6.4.4 Temporal Control

### Table 40 — scene_end_mark object coding (scene_end_mark)

apdu_tag `Tscene_end_mark` = `9F 88 11`, Direction `<---` (EN 50221 §8.6.4.4, Table 40, PDF p. 43).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `scene_end_mark() {` | | |
| &nbsp;&nbsp;scene_end_mark_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;decoder_continue_flag | 1 | bslbf |
| &nbsp;&nbsp;scene_reveal_flag | 1 | bslbf |
| &nbsp;&nbsp;send_scene_done | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;scene_tag | 4 | uimsbf |
| `}` | | |

The application sends a scene end mark after the set of subtitle segments for one
display (the display set) has been sent; it delimits the data set and tells the
decoder what to do once decoding is complete.

- `decoder_continue_flag` — 1 = continue decoding subtitling data (from this MMI
  session); 0 = stop decoding (after any other instructions implied by the mark).
- `scene_reveal_flag` — 1 = implement the immediately preceding page composition
  segment now (display changes to the most-recently-decoded page composition); 0 =
  defer until a scene reveal with the matching `scene_tag` is sent.
- `send_scene_done` — 1 = instruct the decoder to send a scene done APDU to the
  application.
- `scene_tag` — 4-bit integer set by the application; increments modulo 16 per mark.

### Table 41 — scene_done_message coding (scene_done_message)

apdu_tag `Tscene_done` = `9F 88 12`, Direction `<---` (EN 50221 §8.6.4.4, Table 41, PDF p. 43).
Sent by the decoder when it completes decoding a display set followed by a scene end
mark with the send-scene-done flag set.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `scene_done_message() {` | | |
| &nbsp;&nbsp;scene_done_message | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;decoder_continue_flag | 1 | bslbf |
| &nbsp;&nbsp;scene_reveal_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;scene_tag | 4 | uimsbf |
| `}` | | |

- `decoder_continue_flag` / `scene_reveal_flag` — duplicate the state of the same
  flags in the scene end mark that caused this message.
- `scene_tag` — duplicates the integer in the scene end mark that caused this message.

> Render note: Table 41 names the leading 24-bit field `scene_done_message` (the
> same token as the object name), where Tables 40/42 use a `..._tag` suffix.
> Transcribed as printed on p. 43; it is the apdu_tag field (`9F 88 12`). Also note
> the reserved field here is **2 bits** (vs. 1 bit in scene_end_mark), keeping the
> body byte-aligned.

### Table 42 — scene_control coding (scene_control)

apdu_tag `Tscene_control` = `9F 88 13`, Direction `--->` (EN 50221 §8.6.4.4, Table 42, PDF p. 44).
The application may send a scene control APDU only AFTER the scene done message for
the corresponding display set has been sent by the decoder.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `scene_control() {` | | |
| &nbsp;&nbsp;scene_control_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;decoder_continue_flag | 1 | bslbf |
| &nbsp;&nbsp;scene_reveal_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;scene_tag | 4 | uimsbf |
| `}` | | |

- `decoder_continue_flag` — 1 = continue decoding subtitling data; only has effect if
  the decoder continue flag was set to 0 in the scene end mark.
- `scene_reveal_flag` — 1 = implement the new page composition segment for this scene
  subtitling data; only has effect if the scene reveal flag was set to 0 in the scene
  end mark.
- `scene_tag` — 4-bit integer indicating the scene end mark being operated upon;
  increments modulo 16 per scene control.

### §8.6.4.5 Object Download

The subtitle download APDU are identical in format to the subtitle segment APDU, but
are constrained to carry only DVB Subtitling object data segments. The downloaded
object data segment is stored in the object cache; when a region references that
object (by object ID + object provider flag) the segment is read from the cache and
supplied to the decoder input (EN 50221 §8.6.4.5, p. 44).

### Table 43 — subtitle_download coding (subtitle_download)

apdu_tag pair: `Tsubtitle_download_last` = `9F 88 14` / `Tsubtitle_download_more` =
`9F 88 15` (EN 50221 §8.6.4.5, Table 43, PDF p. 45).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `subtitle_download() {` | | |
| &nbsp;&nbsp;subtitle_download_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;DVB_Subtitling_segment() | | |
| `}` | | |

### Table 44 — flush_download coding (flush_download)

apdu_tag `Tflush_download` = `9F 88 16`, Direction `<---` (EN 50221 §8.6.4.5, Table 44, PDF p. 45).
Requests that the decoder's subtitling object cache is purged.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `flush_download() {` | | |
| &nbsp;&nbsp;flush_download_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

### Table 45 — download_reply coding (download_reply)

apdu_tag `Tdownload_reply` = `9F 88 17`, Direction `<---` (EN 50221 §8.6.4.5, Table 45, PDF p. 45).
Allows the host to indicate problems with an object download. Where the download has
been successful there is no requirement to reply.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `download_reply() {` | | |
| &nbsp;&nbsp;download_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;object_id | 16 | uimsbf |
| &nbsp;&nbsp;download_reply_id | 8 | uimsbf |
| `}` | | |

#### download_reply_id values (EN 50221 §8.6.4.5, Table p. 45)

| download_reply_id | value |
|-------------------|-------|
| Download OK              | `00` |
| Not an object data segment | `01` |
| Memory exhausted         | `02` |
| reserved                 | other values |

- `object_id` — the ID of the downloaded object that caused the message. Where the
  message reports that the segment was not an object data segment, `object_id` should
  be `0xFFFF`.

## Last / more chaining

The `_last` / `_more` tag pairs (subtitle_segment, subtitle_download) carry the
**same object body** under two different apdu_tags: the `_last` tag terminates a
fragmented object (last block), the `_more` tag signals more-to-follow. This is the
APDU-layer chaining mechanism (`L_apdu_tag` / `M_apdu_tag`); see `apdu-coding.md`.
The body shown in the table is identical for both tags.

## Cross-reference note on mmi-high-level.md

`mmi-high-level.md` already covers Tables 46-51 (text / enq / answ / menu / menu_answ
/ list) and includes the `choice_nb` field in the Menu (Table 49) and the
`..._last` / `..._more` tag framing for text/menu/list. No omissions of `choice_nb`
or the last/more framing were found in that file.
