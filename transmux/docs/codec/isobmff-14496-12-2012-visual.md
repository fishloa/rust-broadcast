# ISO/IEC 14496-12:2012(E) — Track Header and Visual Sample Entry boxes

Primary-source transcription for display geometry (`docs/display-geometry/DESIGN.md`
§8 Phase 4: `tkhd` presentation size, `pasp`/`clap`/`colr` precedence).

- Source: `private/specs/iso_iec_14496-12_isobmff_2012.pdf` (ISO/IEC 14496-12:2012(E),
  3rd edition, file `C061988e`, 196 pages), markdown
  `private/specs-md/iso_iec_14496-12_isobmff_2012.md` (pdf2md `textlayer`; the converter flags only
  the Introduction p. 11 and Bibliography p. 194 as unverifiable — nothing here comes from them).
- **Every quote and every syntax line below was checked against the rendered PDF page** (pdftoppm +
  visual read), not only the text layer. Page numbers are the printed ISO page numbers
  (PDF page = printed + 12).
- This is the **2012** edition. `isobmff-visual-ext.md` is an excerpt of the **2015** edition
  (§12.1.4/§12.1.5 there; §8.5.2 here). Where both exist the wording is the same for `pasp`/`clap`;
  nothing in the 2015 excerpt contradicts this text.

## §8.3.2 Track Header Box (`tkhd`) — printed pp. 20-22

Box Type `'tkhd'`, Container Track Box (`'trak'`), Mandatory Yes, Quantity Exactly one.

Definition text (p. 20-21), verbatim:

> The default value of the track header flags for media tracks is 7 (track_enabled, track_in_movie,
> track_in_preview). If in a presentation all tracks have neither track_in_movie nor track_in_preview
> set, then all tracks shall be treated as if both flags were set on all tracks. Server hint tracks
> should have the track_in_movie and track_in_preview set to 0, so that they are ignored for local
> playback and preview.
>
> Under the 'iso3' brand or brands that share its requirements, the width and height in the track
> header are measured on a notional 'square' (uniform) grid. Track video data is normalized to these
> dimensions (logically) before any transformation or placement caused by a layup or composition
> system. Track (and movie) matrices, if used, also operate in this uniformly-scaled space.

### Syntax (p. 21)

```
aligned(8) class TrackHeaderBox extends FullBox('tkhd', version, flags){
   if (version==1) {
      unsigned int(64) creation_time;
      unsigned int(64) modification_time;
      unsigned int(32) track_ID;
      const unsigned int(32) reserved = 0;
      unsigned int(64) duration;
   } else { // version==0
      unsigned int(32) creation_time;
      unsigned int(32) modification_time;
      unsigned int(32) track_ID;
      const unsigned int(32) reserved = 0;
      unsigned int(32) duration;
   }
   const unsigned int(32)[2] reserved = 0;
   template int(16) layer = 0;
   template int(16) alternate_group = 0;
   template int(16) volume = {if track_is_audio 0x0100 else 0};
   const unsigned int(16) reserved = 0;
   template int(32)[9] matrix=
      { 0x00010000,0,0,0,0x00010000,0,0,0,0x40000000 };  // unity matrix
   unsigned int(32) width;
   unsigned int(32) height;
}
```

### Semantics (pp. 21-22) — the fields that matter here, verbatim

> flags is a 24-bit integer with flags; the following values are defined:
> Track_enabled: Indicates that the track is enabled. Flag value is 0x000001. A disabled track (the
> low bit is zero) is treated as if it were not present.
> Track_in_movie: Indicates that the track is used in the presentation. Flag value is 0x000002.
> Track_in_preview: Indicates that the track is used when previewing the presentation. Flag value is
> 0x000004.

> matrix provides a transformation matrix for the video; (u,v,w) are restricted here to (0,0,1), hex
> (0,0,0x40000000).

> **width and height specify the track's visual presentation size as fixed-point 16.16 values. These
> need not be the same as the pixel dimensions of the images, which is documented in the sample
> description(s); all images in the sequence are scaled to this size, before any overall
> transformation of the track represented by the matrix. The pixel dimensions of the images are the
> default values.**

(Remaining fields — version, creation_time, modification_time, track_ID, duration, layer,
alternate_group, volume — are on pp. 21-22 of the PDF and are already implemented by
`TrackHeaderBox`; not repeated.)

**What this means for `tkhd` (derived, labelled as such):**
- `width`/`height` are a *presentation* size, scaled-to, independent of the sample-entry pixel
  counts; the sample-entry pixel dimensions are only the *default*.
- Under `iso3`-sharing brands that presentation size is on a **square** grid, i.e. it already
  includes the pixel-aspect correction. The text does not say which brands share `iso3`'s
  requirements; the DVB profile (`isobmff-tkhd.md` §2) states the same square-pixel intent
  normatively for DVB-DASH.
- Not stated by ISO: a rounding rule when `cropped_width × hSpacing / vSpacing` is not an integer
  (16.16 can carry the fraction), and whether `clap` (as opposed to bitstream cropping) is applied
  before the presentation size.

## §8.5.2.3 VisualSampleEntry semantics — printed pp. 29-30

Verbatim (p. 29):

> The width and height in the video sample entry document the pixel counts that the codec will
> deliver; this enables the allocation of buffers. Since these are counts they do not take into
> account pixel aspect ratio.
>
> The pixel aspect ratio and clean aperture of the video may be specified using the 'pasp' and
> 'clap' sample entry boxes, respectively. These are both optional; **if present, they over-ride the
> declarations (if any) in structures specific to the video codec, which structures should be
> examined if these boxes are absent.** For maximum compatibility, these boxes should follow, not
> precede, any boxes defined in or required by derived specifications.
>
> In the PixelAspectRatioBox, hSpacing and vSpacing have the same units, but those units are
> unspecified: only the ratio matters. hSpacing and vSpacing may or may not be in reduced terms, and
> they may reduce to 1/1. Both of them must be positive.
>
> They are defined as the aspect ratio of a pixel, in arbitrary units. If a pixel appears H wide and V
> tall, then hSpacing/vSpacing is equal to H/V. This means that a square on the display that is n
> pixels tall needs to be n*vSpacing/hSpacing pixels wide to appear square.
>
> NOTE When adjusting pixel aspect ratio, normally, the horizontal dimension of the video is scaled,
> if needed (i.e. if the final display system has a different pixel aspect ratio from the video
> source).
>
> NOTE It is recommended that the original pixels, and the composed transform, be carried through the
> pipeline as far as possible. If the transformation resulting from 'correcting' pixel aspect ratio to
> a square grid, normalizing to the track dimensions, composition or placement (e.g. track and/or
> movie matrix), and normalizing to the display characteristics, is a unity matrix, then no
> re-sampling need be done. In particular, video should not be re-sampled more than once in the
> process of rendering, if at all possible.

Verbatim (p. 30):

> There are notionally four values in the CleanApertureBox. These parameters are represented as a
> fraction N/D. The fraction may or may not be in reduced terms. We refer to the pair of parameters
> fooN and fooD as foo. For horizOff and vertOff, D must be positive and N may be positive or
> negative. For cleanApertureWidth and cleanApertureHeight, both N and D must be positive.
>
> NOTE These are fractional numbers for several reasons. First, in some systems the exact width after
> pixel aspect ratio correction is integral, not the pixel count before that correction. Second, if
> video is resized in the full aperture, the exact expression for the clean aperture may not be
> integral. Finally, because this is represented using centre and offset, a division by two is
> needed, and so half-values can occur.
>
> Considering the pixel dimensions as defined by the VisualSampleEntry width and height. If picture
> centre of the image is at pcX and pcY, then horizOff and vertOff are defined as follows:
>
> pcX = horizOff + (width - 1)/2
> pcY = vertOff  + (height - 1)/2;
>
> Typically, horizOff and vertOff are zero, so the image is centred about the picture centre.
>
> The leftmost/rightmost pixel and the topmost/bottommost line of the clean aperture fall at:
>
> pcX ± (cleanApertureWidth - 1)/2
> pcY ± (cleanApertureHeight - 1)/2;

Colour information (p. 30), verbatim:

> Colour information may be supplied in one or more ColourInformationBoxes placed in a
> VisualSampleEntry. These should be placed in order in the sample entry starting with the most
> accurate (and potentially the most difficult to process), in progression to the least. These are
> advisory and concern rendering and colour conversion, and there is no normative behaviour
> associated with them; a reader may choose to use the most suitable. A ColourInformationBox with an
> unknown colour type may be ignored.
>
> If used, an ICC profile may be a restricted one, under the code 'rICC', which permits simpler
> processing. That profile shall be of either the Monochrome or Three-Component Matrix-Based class of
> input profiles, as defined by ISO 15076-1. If the profile is of another class, then the 'prof'
> indicator must be used.
>
> **If colour information is supplied in both this box, and also in the video bitstream, this box
> takes precedence, and over-rides the information in the bitstream.**

### Syntax (p. 32)

```
class PixelAspectRatioBox extends Box('pasp'){
   unsigned int(32) hSpacing;
   unsigned int(32) vSpacing;
}
class CleanApertureBox extends Box('clap'){
   unsigned int(32) cleanApertureWidthN;
   unsigned int(32) cleanApertureWidthD;
   unsigned int(32) cleanApertureHeightN;
   unsigned int(32) cleanApertureHeightD;
   unsigned int(32) horizOffN;
   unsigned int(32) horizOffD;
   unsigned int(32) vertOffN;
   unsigned int(32) vertOffD;
}
class ColourInformationBox extends Box('colr'){
   unsigned int(32) colour_type;
   if (colour_type == 'nclx') /* on-screen colours */
   {
      unsigned int(16) colour_primaries;
      unsigned int(16) transfer_characteristics;
      unsigned int(16) matrix_coefficients;
      unsigned int(1)  full_range_flag;
      unsigned int(7)  reserved = 0;
   }
   else if (colour_type == 'rICC') { ICC_profile; }  // restricted ICC profile
   else if (colour_type == 'prof') { ICC_profile; }  // unrestricted ICC profile
}
class VisualSampleEntry(codingname) extends SampleEntry (codingname){
   unsigned int(16) pre_defined = 0;
   const unsigned int(16) reserved = 0;
   unsigned int(32)[3] pre_defined = 0;
   unsigned int(16) width;
   unsigned int(16) height;
   template unsigned int(32) horizresolution = 0x00480000; // 72 dpi
   template unsigned int(32) vertresolution  = 0x00480000; // 72 dpi
   const unsigned int(32) reserved = 0;
   template unsigned int(16) frame_count = 1;
   string[32] compressorname;
   template unsigned int(16) depth = 0x0018;
   int(16) pre_defined = -1;
   // other boxes from derived specifications
   CleanApertureBox     clap; // optional
   PixelAspectRatioBox  pasp; // optional
}
```

## What this settles (previously gap G3 in DESIGN.md §14)

1. **`pasp`/`clap` vs the codec's own aspect/crop information:** the boxes win; the codec-specific
   structures (SPS VUI aspect ratio, SPS crop) are used only when the boxes are absent.
2. **`colr` vs bitstream colour (VUI):** the box wins and overrides the bitstream.
3. **`pasp` absence** means "examine the codec structures", NOT "square pixels" — a reader that has
   a VUI SAR uses it when `pasp` is missing.
4. **Presentation size:** `tkhd` width/height (above).

## Still not covered by any available source

- The semantics of the `nclx` code points themselves (ISO/IEC 23001-8; the tables are in
  `vui-colour-tables.md` from H.264/H.265 Annex E).
- How `clap` interacts with `tkhd` presentation size and with SPS cropping (ISO says boxes override
  codec structures; the order in which crop → SAR rescale → `tkhd` is applied is not specified).
- ISO/IEC 23009-1 (DASH) `@sar`/`@par` attribute semantics beyond ETSI TS 103 285 §4.4.
