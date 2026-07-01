# Subtitle/caption carriage in ISOBMFF (ISO/IEC 14496-30) — #430

**Source:** 14496-30 is paid; `stpp` structure below matches the **real fixture**
`fixtures/mp4/stpp.mp4` (ffmpeg `ttml` encoder). `wvtt` (WebVTT-in-ISOBMFF) can't be
produced by ffmpeg's mp4 muxer and no GPAC is installed → cover with a **spec vector**
built from the structure below; the field layout is the well-established 14496-30 form.

## stpp — XMLSubtitleSampleEntry (TTML / IMSC)  ✅ real fixture

```
class XMLSubtitleSampleEntry extends SampleEntry('stpp') {
    string namespace;              // null-terminated, space-separated XML namespaces
    string schema_location;        // null-terminated (may be empty)
    string auxiliary_mime_types;   // null-terminated (may be empty)
    // optional: BitRateBox, ...
}
```
Samples are whole TTML documents (XML). Handler type `subt`.

## wvtt — WVTTSampleEntry (WebVTT)  ⚠️ spec-vector

```
class WVTTSampleEntry extends SampleEntry('wvtt') {
    WebVTTConfigurationBox   config;   // 'vttC'
    WebVTTSourceLabelBox     label;    // 'vlab', optional
    // optional: MPEG4BitRateBox 'btrt'
}
class WebVTTConfigurationBox extends Box('vttC') { string config; }        // WebVTT header block
class WebVTTSourceLabelBox   extends Box('vlab') { string source_label; }
```
Cue sample payload:
```
class VTTCueBox extends Box('vttc') {         // one per presented cue
    CueSourceIDBox   ('vsid')  // optional, unsigned int(32) source_ID
    CueIDBox         ('iden')  // optional, string cue_id
    CueTimeBox       ('ctim')  // optional, string cue_current_time
    CueSettingsBox   ('sttg')  // optional, string settings
    CuePayloadBox    ('payl')  // required, string cue_text
}
class VTTEmptyCueBox extends Box('vtte') { }   // gap sample (no active cue)
class VTTAdditionalTextBox extends Box('vtta') { string cue_additional_text }
```
Handler type `text`. Init `vttC.config` = the file's WebVTT header (e.g. "WEBVTT").
