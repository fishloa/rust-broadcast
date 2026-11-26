# Video frame seam review

On-paper comparison only. No video decoder adapter is built in this lane.

`VideoFrame` / `PixelFormat` / `VideoStorage` are checked against the decoded
picture types of two real decoders, read from their published API documentation:

- **rav1d 1.1.0** — `rav1d::include::dav1d::picture::Dav1dPicture` and the
  `headers` types it points at (`Rav1dSequenceHeader`, `Rav1dFrameHeader`,
  `Rav1dMasteringDisplay`, `Rav1dContentLightLevel`, `Rav1dITUTT35`).
  <https://docs.rs/rav1d/1.1.0/rav1d/include/dav1d/picture/struct.Dav1dPicture.html>
- **openh264 0.9.10** — `openh264::decoder::DecodedYUV` and the `YUVSource`
  trait it implements.
  <https://docs.rs/openh264/0.9.10/openh264/decoder/struct.DecodedYUV.html>

Verdicts: **fits** (our types already carry it), **needs a field** (our frame
is missing it), **needs a variant** (our pixel-format vocabulary cannot name
it). Every "needs" is implemented in `broadcast-codec`; those changes are
listed at the end.

## rav1d (`Dav1dPicture`)

| Their field or concept | Ours | Verdict |
|---|---|---|
| `p.w`, `p.h` (`c_int`) — picture width and height | `VideoFrame::width()` / `height()` (`NonZeroU32`) | fits |
| `p.layout`: `I400`, `I420`, `I422`, `I444` | `PixelFormat` monochrome / `Yuv420p*` / `Yuv422p*` / `Yuv444p*` | fits |
| `p.bpc`: 8, 10 or 12 bits per sample | `PixelFormat` names the depth, but only 8 and 10 exist, and monochrome exists only as `Gray8` | **needs a variant** |
| `data[3]` plane pointers | `VideoStorage::Cpu(Vec<VideoPlane>)` | fits (any plane count) |
| `stride[2]` — luma stride and one shared chroma stride | `VideoPlane::stride()` per plane | fits (we are more general, including a padded last row) |
| `m.timestamp` (`i64`) | `VideoFrame::pts()` (`Option<i64>` in the stream time base) | fits |
| `m.duration` (`i64`) | none | fits — frame duration is derived from the caller's timeline; the seam deliberately carries `pts` only |
| `frame_hdr.frame_type`: `Key`, `Inter`, `Intra`, `Switch` | `VideoFrame::key()` | fits — `key` is the random-access signal; `Switch` is not a random-access point |
| `seq_hdr.pri` / `.trc` / `.mtrx` / `.color_range`, `color_description_present` | `ColourDescription` plus `Option` for presence | fits |
| `content_light` (`Rav1dContentLightLevel`: `max_content_light_level`, `max_frame_average_light_level`) | none | **needs a field** |
| `mastering_display` (`Rav1dMasteringDisplay`: `primaries`, `white_point`, `max_luminance`, `min_luminance`) | none | **needs a field** |
| `itut_t35` / `n_itut_t35` (`Rav1dITUTT35`: `country_code`, `country_code_extension_byte`, `payload`) | none | **needs a field** |

## openh264 (`DecodedYUV`)

| Their field or concept | Ours | Verdict |
|---|---|---|
| `dimensions()` `(w, h)` | `VideoFrame::width()` / `height()` | fits |
| `dimensions_uv()` — unpadded chroma size | derived by `PixelFormat` plane geometry | fits |
| `strides()` `(y, u, v)` | `VideoPlane::stride()` per plane | fits |
| `y()`, `u()`, `v()` byte slices | `VideoStorage::Cpu` with three planes | fits |
| 4:2:0 byte samples | `PixelFormat::Yuv420p8` | fits |
| `timestamp()` — milliseconds relative to stream start | `VideoFrame::pts()` in the stream time base; an adapter converts | fits |
| no key-frame accessor on `DecodedYUV` | `VideoFrame::key()` stays `false` | fits — the library gives no signal to carry |
| `write_rgb8` / `write_rgba8` | conversion helpers, not decoded storage; `Rgb24` / `Rgba32` exist | fits |
| no colour description or HDR metadata | `colour()`, `content_light()`, `mastering_display()` stay `None` | fits |

## Changes made

Both came from the rav1d table; openh264 forced none.

- **`PixelFormat` variants** `Gray10`, `Gray12`, `Yuv420p12`, `Yuv422p12`,
  `Yuv444p12`, with their plane geometry (12-bit samples use the same 16-bit
  container as 10-bit ones). This names everything `p.layout` × `p.bpc` can
  produce for AV1.
- **`ContentLightLevel`**, **`MasteringDisplay`** and **`ItuT35`** structs, and
  `VideoFrame` fields built by `with_content_light`, `with_mastering_display`
  and `with_itu_t35`, read back through the matching accessors. `ItuT35`
  keeps its payload opaque, like `CodecParameters::extradata`.

## OPEN

- rav1d's API documentation is sparse (roughly 9% documented) and most items
  have no accessor methods; the fields above are the public `#[repr(C)]`
  fields quoted from the generated signatures. No API was guessed.
- `openh264::Timestamp` documentation itself writes "milliseconds(?)"; the
  unit is stated as milliseconds by `from_millis` / `as_millis`, and an
  adapter maps it into the stream time base.
