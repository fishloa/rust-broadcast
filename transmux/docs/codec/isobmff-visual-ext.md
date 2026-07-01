absent. For maximum compatibility, these boxes should follow, not precede, any boxes defined in or required by derived specifications. 

In the PixelAspectRatioBox, hSpacing and vSpacing have the same units, but those units are unspecified: only the ratio matters. hSpacing and vSpacing may or may not be in reduced terms, and they may reduce to 1/1. Both of them must be positive. 

They are defined as the aspect ratio of a pixel, in arbitrary units. If a pixel appears H wide and V tall, then hSpacing/vSpacing is equal to H/V. This means that a square on the display that is n pixels tall needs to be n\*vSpacing/hSpacing pixels wide to appear square. 

NOTE When adjusting pixel aspect ratio, normally, the horizontal dimension of the video is scaled, if needed (i.e. if the final display system has a different pixel aspect ratio from the video source). 

NOTE It is recommended that the original pixels, and the composed transform, be carried through the pipeline as far as possible. If the transformation resulting from 'correcting' pixel aspect ratio to a square grid, normalizing to the track dimensions, composition or placement (e.g. track and/or movie matrix), and normalizing to the display characteristics, is a unity matrix, then no re‐sampling need be done. In particular, video should not be re‐ sampled more than once in the process of rendering, if at all possible. 

There are notionally four values in the CleanApertureBox. These parameters are represented as a fraction N/D. The fraction may or may not be in reduced terms. We refer to the pair of parameters fooN and fooD as foo. For horizOff and vertOff, D must be positive and N may be positive or negative. For cleanApertureWidth and cleanApertureHeight, both N and D must be positive. 

NOTE These are fractional numbers for several reasons. First, in some systems the exact width after pixel aspect ratio correction is integral, not the pixel count before that correction. Second, if video is resized in the full aperture, the exact expression for the clean aperture may not be integral. Finally, because this is represented using centre and offset, a division by two is needed, and so half‐values can occur. 

Considering the pixel dimensions as defined by the VisualSampleEntry width and height. If picture centre of the image is at pcX and pcY, then horizOff and vertOff are defined as follows: 

```
pcX = horizOff + (width - 1)/2 
pcY = vertOff + (height - 1)/2;
```

Typically, horizOff and vertOff are zero, so the image is centred about the picture centre. 

The leftmost/rightmost pixel and the topmost/bottommost line of the clean aperture fall at: 

```
pcX ± (cleanApertureWidth - 1)/2 
pcY ± (cleanApertureHeight - 1)/2; 
12.1.4.2 Syntax 
class PixelAspectRatioBox extends Box('pasp'){ 
 unsigned int(32) hSpacing; 
 unsigned int(32) vSpacing; 
}
```

# **ISO/IEC 14496-12:2015(E)**

```
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
```

## **12.1.4.3 Semantics**

hSpacing, vSpacing: define the relative width and height of a pixel; 

- cleanApertureWidthN, cleanApertureWidthD: a fractional number which defines the exact clean aperture width, in counted pixels, of the video image
- cleanApertureHeightN, cleanApertureHeightD: a fractional number which defines the exact clean aperture height, in counted pixels, of the video image
- horizOffN, horizOffD: a fractional number which defines the horizontal offset of clean aperture centre minus (width‐1)/2. Typically 0.
- vertOffN, vertOffD: a fractional number which defines the vertical offset of clean aperture centre minus (height‐1)/2. Typically 0.

## **12.1.5 Colour information**

## **12.1.5.1 Definition**

Colour information may be supplied in one or more ColourInformationBoxes placed in a VisualSampleEntry. These should be placed in order in the sample entry starting with the most accurate (and potentially the most difficult to process), in progression to the least. These are advisory and concern rendering and colour conversion, and there is no normative behaviour associated with them; a reader may choose to use the most suitable. A ColourInformationBox with an unknown colour type may be ignored. 

If used, an ICC profile may be a restricted one, under the code 'rICC', which permits simpler processing. That profile shall be of either the Monochrome or Three‐Component Matrix‐Based class of input profiles, as defined by ISO 15076‐1. If the profile is of another class, then the 'prof' indicator must be used. 

If colour information is supplied in both this box, and also in the video bitstream, this box takes precedence, and over‐rides the information in the bitstream. 

NOTE When an ICC profile is specified, SMPTE RP 177 "Derivation of Basic Television Color Equations" may be of assistance if there is a need to form the Y'CbCr to R'G'B' conversion matrix for the color primaries described by the ICC profile.

## **12.1.5.2 Syntax**

```
class ColourInformationBox extends Box('colr'){ 
 unsigned int(32) colour_type; 
 if (colour_type == 'nclx') /* on-screen colours */ 
 { 
 unsigned int(16) colour_primaries; 
 unsigned int(16) transfer_characteristics; 
 unsigned int(16) matrix_coefficients; 
 unsigned int(1) full_range_flag; 
 unsigned int(7) reserved = 0; 
 } 
 else if (colour_type == 'rICC') 
 { 
 ICC_profile; // restricted ICC profile 
 } 
 else if (colour_type == 'prof') 
 { 
 ICC_profile; // unrestricted ICC profile 
 } 
}
```

## **12.1.5.3 Semantics**

colour\_type: an indication of the type of colour information supplied. For colour\_type 'nclx': these fields are exactly the four bytes defined for PTM\_COLOR\_INFO( ) in A.7.2 of ISO/IEC 29199‐2 but note that the full range flag is here in a different bit position ICC\_profile: an ICC profile as defined in ISO 15076‐1 or ICC.1:2010 is supplied. 

# **12.2 Audio media**

## **12.2.1 Media handler**

Audio media uses the 'soun' handler type in the handler box of the media box, as defined in 8.4.3. 

## **12.2.2 Sound media header**

# **12.2.2.1 Definition**

Box Types: 'smhd' 

Container: Media Information Box ('minf') 

Mandatory: Yes 

Quantity: Exactly one specific media header shall be present 

Audio tracks use the SoundMediaHeaderbox in the media information box as defined in 8.4.5. The sound media header contains general presentation information, independent of the coding, for audio media. This header is used for all tracks containing audio. 

## **12.2.2.2 Syntax**

```
aligned(8) class SoundMediaHeaderBox 
 extends FullBox('smhd', version = 0, 0) { 
 template int(16) balance = 0; 
 const unsigned int(16) reserved = 0; 
}
```

# **12.2.2.3 Semantics**

version is an integer that specifies the version of this box

# **ISO/IEC 14496-12:2015(E)**

balance is a fixed‐point 8.8 number that places mono audio tracks in a stereo space; 0 is centre (the normal value); full left is ‐1.0 and full right is 1.0. 

# **12.2.3 Sample entry**

## **12.2.3.1 Definition**

Audio tracks use AudioSampleEntry or AudioSampleEntryV1. 

The samplerate, samplesize and channelcount fields document the default audio output playback format for this media. The timescale for an audio track should be chosen to match the sampling rate, or be an integer multiple of it, to enable sample‐accurate timing. When channelcount is a value greater than zero, it indicates the intended number of loudspeaker channels in the audio stream. A ChannelCount of 1 indicates mono audio, and 2 indicates stereo (left/right). When values greater than 2 are used, the codec configuration should identify the channel assignment. 

When it is desired to indicate an audio sampling rate greater than the value that can be represented in the samplerate field, the following may be used: 

- an AudioSampleEntryV1 is used, which requires that the enclosing Sample Description Box also take the version 1;
- a Sampling Rate box may be present only in an AudioSampleEntryV1, and when present, it over‐ rides the samplerate field and documents the actual sampling rate;
- when the Sampling Rate box is present, the media timescale should be the same as the sampling rate, or an integer division or multiple of it;
- the samplerate field in the sample entry should contain a value left‐shifted 16 bits (as for AudioSampleEntry) that matches the media timescale, or be an integer division or multiple of it.

An AudioSampleEntryV1 should only be used when needed; otherwise, for maximum compatibility, an AudioSampleEntry should be used. An AudioSampleEntryV1 must not occur in a SampleDescriptionBox with version set to 0. 

The audio output format (samplerate, samplesize and channelcount fields) in the sample entry should be considered definitive only for codecs that do not record their own output configuration. If the audio codec has definitive information about the output format, it shall be taken as definitive; in this case the samplerate, samplesize and channelcount fields in the sample entry may be ignored, though sensible values should be chosen (for example, the highest possible sampling rate).