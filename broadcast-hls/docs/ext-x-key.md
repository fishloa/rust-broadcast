# EXT-X-KEY, key files, IV and segment encryption (RFC 8216)

Verbatim from `specs/ietf_rfc8216_hls.txt` (plain-text RFC, no OCR; page header/footer lines removed). Binding source for issue #1459.

## RFC 8216 section 4.3.2.4 EXT-X-KEY

```text
4.3.2.4.  EXT-X-KEY

   Media Segments MAY be encrypted.  The EXT-X-KEY tag specifies how to
   decrypt them.  It applies to every Media Segment and to every Media
   Initialization Section declared by an EXT-X-MAP tag that appears
   between it and the next EXT-X-KEY tag in the Playlist file with the
   same KEYFORMAT attribute (or the end of the Playlist file).  Two or
   more EXT-X-KEY tags with different KEYFORMAT attributes MAY apply to
   the same Media Segment if they ultimately produce the same decryption
   key.  The format is:

   #EXT-X-KEY:<attribute-list>

   The following attributes are defined:

      METHOD

      The value is an enumerated-string that specifies the encryption
      method.  This attribute is REQUIRED.

      The methods defined are: NONE, AES-128, and SAMPLE-AES.

      An encryption method of NONE means that Media Segments are not
      encrypted.  If the encryption method is NONE, other attributes
      MUST NOT be present.

      An encryption method of AES-128 signals that Media Segments are
      completely encrypted using the Advanced Encryption Standard (AES)
      [AES_128] with a 128-bit key, Cipher Block Chaining (CBC), and
      Public-Key Cryptography Standards #7 (PKCS7) padding [RFC5652].
      CBC is restarted on each segment boundary, using either the
      Initialization Vector (IV) attribute value or the Media Sequence
      Number as the IV; see Section 5.2.

      An encryption method of SAMPLE-AES means that the Media Segments
      contain media samples, such as audio or video, that are encrypted
      using the Advanced Encryption Standard [AES_128].  How these media
      streams are encrypted and encapsulated in a segment depends on the

      media encoding and the media format of the segment.  fMP4 Media
      Segments are encrypted using the 'cbcs' scheme of Common
      Encryption [COMMON_ENC].  Encryption of other Media Segment
      formats containing H.264 [H_264], AAC [ISO_14496], AC-3 [AC_3],
      and Enhanced AC-3 [AC_3] media streams is described in the HTTP
      Live Streaming (HLS) Sample Encryption specification [SampleEnc].
      The IV attribute MAY be present; see Section 5.2.

      URI

      The value is a quoted-string containing a URI that specifies how
      to obtain the key.  This attribute is REQUIRED unless the METHOD
      is NONE.

      IV

      The value is a hexadecimal-sequence that specifies a 128-bit
      unsigned integer Initialization Vector to be used with the key.
      Use of the IV attribute REQUIRES a compatibility version number of
      2 or greater.  See Section 5.2 for when the IV attribute is used.

      KEYFORMAT

      The value is a quoted-string that specifies how the key is
      represented in the resource identified by the URI; see Section 5
      for more detail.  This attribute is OPTIONAL; its absence
      indicates an implicit value of "identity".  Use of the KEYFORMAT
      attribute REQUIRES a compatibility version number of 5 or greater.

      KEYFORMATVERSIONS

      The value is a quoted-string containing one or more positive
      integers separated by the "/" character (for example, "1", "1/2",
      or "1/2/5").  If more than one version of a particular KEYFORMAT
      is defined, this attribute can be used to indicate which
      version(s) this instance complies with.  This attribute is
      OPTIONAL; if it is not present, its value is considered to be "1".
      Use of the KEYFORMATVERSIONS attribute REQUIRES a compatibility
      version number of 5 or greater.

   If the Media Playlist file does not contain an EXT-X-KEY tag, then
   Media Segments are not encrypted.
```

## RFC 8216 section 5 Key Files / 5.2 IV for AES-128

```text
5.1.  Structure of Key Files

   An EXT-X-KEY tag with a URI attribute identifies a Key file.  A Key
   file contains a cipher key that can decrypt Media Segments in the
   Playlist.

   [AES_128] encryption uses 16-octet keys.  If the KEYFORMAT of an EXT-
   X-KEY tag is "identity", the Key file is a single packed array of 16
   octets in binary format.

5.2.  IV for AES-128

   [AES_128] REQUIRES the same 16-octet IV to be supplied when
   encrypting and decrypting.  Varying this IV increases the strength of
   the cipher.

   An IV attribute on an EXT-X-KEY tag with a KEYFORMAT of "identity"
   specifies an IV that can be used when decrypting Media Segments
   encrypted with that Key file.  IV values for AES-128 are 128-bit
   numbers.

   An EXT-X-KEY tag with a KEYFORMAT of "identity" that does not have an
   IV attribute indicates that the Media Sequence Number is to be used
   as the IV when decrypting a Media Segment, by putting its big-endian
   binary representation into a 16-octet (128-bit) buffer and padding
   (on the left) with zeros.
```

## RFC 8216 section 6.2.3 Encrypting Media Segments

```text
6.2.3.  Encrypting Media Segments

   Media Segments MAY be encrypted.  Every encrypted Media Segment MUST
   have an EXT-X-KEY tag (Section 4.3.2.4) applied to it with a URI that
   the client can use to obtain a Key file (Section 5) containing the
   decryption key.

   A Media Segment can only be encrypted with one encryption METHOD,
   using one encryption key and IV.  However, a server MAY offer
   multiple ways to retrieve that key by providing multiple EXT-X-KEY
   tags, each with a different KEYFORMAT attribute value.

   The server MAY set the HTTP Expires header in the key response to
   indicate the duration for which the key can be cached.

   Any unencrypted Media Segment in a Playlist that is preceded by an
   encrypted Media Segment MUST have an EXT-X-KEY tag applied to it with
   a METHOD attribute of NONE.  Otherwise, the client will misinterpret
   those segments as encrypted.

   If the encryption METHOD is AES-128 and the Playlist does not contain
   the EXT-X-I-FRAMES-ONLY tag, AES encryption as described in
   Section 4.3.2.4 SHALL be applied to individual Media Segments.

   If the encryption METHOD is AES-128 and the Playlist contains an EXT-
   X-I-FRAMES-ONLY tag, the entire resource MUST be encrypted using
   AES-128 CBC with PKCS7 padding [RFC5652].  Encryption MAY be
   restarted on 16-byte block boundaries, unless the first block
   contains an I-frame.  The IV used for encryption MUST be either the
   Media Sequence Number of the Media Segment or the value of the IV
   attribute of the EXT-X-KEY tag, as described in Section 5.2.  These
   constraints allow a client to load and decrypt individual I-frames
   specified as sub-ranges of regular encrypted Media Segments, and
   their Media Initialization Sections.

   If the encryption METHOD is SAMPLE-AES, media samples MAY be
   encrypted prior to encapsulation in a Media Segment.

   The server MUST NOT remove an EXT-X-KEY tag from the Playlist file if
   it applies to any Media Segment in the Playlist file, or clients who
   subsequently load that Playlist will be unable to decrypt those Media
   Segments.

6.2.4.  Providing Variant Streams

   A server MAY offer multiple Media Playlist files to provide different
   encodings of the same presentation.  If it does so, it SHOULD provide
   a Master Playlist file that lists each Variant Stream to allow
   clients to switch between encodings dynamically.

   Master Playlists describe regular Variant Streams with EXT-X-STREAM-
   INF tags and I-frame Variant Streams with EXT-X-I-FRAME-STREAM-INF
   tags.
```

## Decisions for the implementation

- METHOD=NONE: segments plain. AES-128: whole segment AES-128-CBC, PKCS7 padding, CBC restarted at each segment; key = 16 raw octets from the URI body (KEYFORMAT identity); IV = the IV attribute (hex, 128-bit) or else the Media Sequence Number of that segment as a 16-octet big-endian number left-padded with zeros. SAMPLE-AES (TS, H.264/AAC/AC-3/E-AC-3 per the Sample Encryption spec; fMP4 = CENC `cbcs`) is OUT of the first slice: a segment under SAMPLE-AES yields a typed `Unsupported` failure, never silent plaintext; transmux has the primitives (`sample_aes`), wiring is a later story.
- An EXT-X-KEY applies to every following segment and EXT-X-MAP until the next EXT-X-KEY with the same KEYFORMAT. KEYFORMAT other than `identity` (e.g. FairPlay `com.apple.streamingkeydelivery`) is not decryptable here: typed `Unsupported`.
- EXT-X-I-FRAMES-ONLY: the whole resource is encrypted; sub-range requests are an out-of-scope case (typed `Unsupported`).
- Key fetch: the key URI is a normal HTTP resource; in broadcast-client it is fetched as an `HttpRequest` through the same auth/redirect layer as everything else (credentials only to the session origin), cached per URI for the life of the session (Expires header ignored in the first slice), a key response that is not exactly 16 bytes is a failure.
