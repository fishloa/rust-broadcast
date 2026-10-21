# HTTP range requests — RFC 9110 transcription (for serving I-frame `EXT-X-BYTERANGE` entries)

Source: RFC 9110 "HTTP Semantics" (June 2022), `specs/rfc9110.txt` (fetched from rfc-editor.org
2026-10-22). Verbatim blocks; line numbers count `\n`-separated lines of that file (page-break form
feeds retained inside blocks where they occur). Written BEFORE implementation, as the basis for the
`Range` support the multimux origin needs so that clients can fetch the byte ranges named by
`EXT-X-BYTERANGE` (RFC 8216bis §4.4.4.2; `broadcast-hls/docs/iframe-playlists.md` T§3).

Why this exists: `multimux`'s HTTP origin currently serves whole resources only (`Range` appears only in
the CORS allow-list, `multimux/src/origin/mod.rs`); a client following an I-frame playlist sends
`Range: bytes=0-<n>` and must get `206 Partial Content` with a matching `Content-Range`.

## 13.1.5.  If-Range (lines 6109-6180)

```
13.1.5.  If-Range

   The "If-Range" header field provides a special conditional request
   mechanism that is similar to the If-Match and If-Unmodified-Since
   header fields but that instructs the recipient to ignore the Range
   header field if the validator doesn't match, resulting in transfer of
   the new selected representation instead of a 412 (Precondition
   Failed) response.

   If a client has a partial copy of a representation and wishes to have
   an up-to-date copy of the entire representation, it could use the
   Range header field with a conditional GET (using either or both of
   If-Unmodified-Since and If-Match.)  However, if the precondition
   fails because the representation has been modified, the client would
   then have to make a second request to obtain the entire current
   representation.

   The "If-Range" header field allows a client to "short-circuit" the
   second request.  Informally, its meaning is as follows: if the
   representation is unchanged, send me the part(s) that I am requesting
   in Range; otherwise, send me the entire representation.

     If-Range = entity-tag / HTTP-date

   A valid entity-tag can be distinguished from a valid HTTP-date by
   examining the first three characters for a DQUOTE.

   A client MUST NOT generate an If-Range header field in a request that
   does not contain a Range header field.  A server MUST ignore an If-
   Range header field received in a request that does not contain a
   Range header field.  An origin server MUST ignore an If-Range header
   field received in a request for a target resource that does not
   support Range requests.

   A client MUST NOT generate an If-Range header field containing an
   entity tag that is marked as weak.  A client MUST NOT generate an If-
   Range header field containing an HTTP-date unless the client has no
   entity tag for the corresponding representation and the date is a
   strong validator in the sense defined by Section 8.8.2.2.

   A server that receives an If-Range header field on a Range request
   MUST evaluate the condition per Section 13.2 prior to performing the
   method.

   To evaluate a received If-Range header field containing an HTTP-date:

   1.  If the HTTP-date validator provided is not a strong validator in
       the sense defined by Section 8.8.2.2, the condition is false.

   2.  If the HTTP-date validator provided exactly matches the
       Last-Modified field value for the selected representation, the
       condition is true.

   3.  Otherwise, the condition is false.

   To evaluate a received If-Range header field containing an
   entity-tag:

   1.  If the entity-tag validator provided exactly matches the ETag
       field value for the selected representation using the strong
       comparison function (Section 8.8.3.2), the condition is true.

   2.  Otherwise, the condition is false.

   A recipient of an If-Range header field MUST ignore the Range header
   field if the If-Range condition evaluates to false.  Otherwise, the
   recipient SHOULD process the Range header field as requested.

   Note that the If-Range comparison is by exact match, including when
   the validator is an HTTP-date, and so it differs from the "earlier
   than or equal to" comparison used when evaluating an
   If-Unmodified-Since conditional.
```

## 14.1.  Range Units (lines 6308-6330)

```
14.1.  Range Units

   Representation data can be partitioned into subranges when there are
   addressable structural units inherent to that data's content coding
   or media type.  For example, octet (a.k.a. byte) boundaries are a
   structural unit common to all representation data, allowing
   partitions of the data to be identified as a range of bytes at some
   offset from the start or end of that data.

   This general notion of a "range unit" is used in the Accept-Ranges
   (Section 14.3) response header field to advertise support for range
   requests, the Range (Section 14.2) request header field to delineate
   the parts of a representation that are requested, and the
   Content-Range (Section 14.4) header field to describe which part of a
   representation is being transferred.

     range-unit       = token

   All range unit names are case-insensitive and ought to be registered
   within the "HTTP Range Unit Registry", as defined in Section 16.5.1.

   Range units are intended to be extensible, as described in
   Section 16.5.
```

## 14.1.1.  Range Specifiers (lines 6332-6382)

```
14.1.1.  Range Specifiers

   Ranges are expressed in terms of a range unit paired with a set of
   range specifiers.  The range unit name determines what kinds of
   range-spec are applicable to its own specifiers.  Hence, the
   following grammar is generic: each range unit is expected to specify
   requirements on when int-range, suffix-range, and other-range are
   allowed.

   A range request can specify a single range or a set of ranges within
   a single representation.

     ranges-specifier = range-unit "=" range-set
     range-set        = 1#range-spec
     range-spec       = int-range
                      / suffix-range
                      / other-range

   An int-range is a range expressed as two non-negative integers or as
   one non-negative integer through to the end of the representation
   data.  The range unit specifies what the integers mean (e.g., they
   might indicate unit offsets from the beginning, inclusive numbered
   parts, etc.).

     int-range     = first-pos "-" [ last-pos ]
     first-pos     = 1*DIGIT
     last-pos      = 1*DIGIT

   An int-range is invalid if the last-pos value is present and less
   than the first-pos.

   A suffix-range is a range expressed as a suffix of the representation
   data with the provided non-negative integer maximum length (in range
   units).  In other words, the last N units of the representation data.

     suffix-range  = "-" suffix-length
     suffix-length = 1*DIGIT

   To provide for extensibility, the other-range rule is a mostly
   unconstrained grammar that allows application-specific or future
   range units to define additional range specifiers.

     other-range   = 1*( %x21-2B / %x2D-7E )
                   ; 1*(VCHAR excluding comma)

   A ranges-specifier is invalid if it contains any range-spec that is
   invalid or undefined for the indicated range-unit.

   A valid ranges-specifier is "satisfiable" if it contains at least one
   range-spec that is satisfiable, as defined by the indicated
   range-unit.  Otherwise, the ranges-specifier is "unsatisfiable".
```

## 14.1.2.  Byte Ranges (lines 6384-6465)

```
14.1.2.  Byte Ranges

   The "bytes" range unit is used to express subranges of a
   representation data's octet sequence.  Each byte range is expressed
   as an integer range at some offset, relative to either the beginning
   (int-range) or end (suffix-range) of the representation data.  Byte
   ranges do not use the other-range specifier.

   The first-pos value in a bytes int-range gives the offset of the
   first byte in a range.  The last-pos value gives the offset of the
   last byte in the range; that is, the byte positions specified are
   inclusive.  Byte offsets start at zero.

   If the representation data has a content coding applied, each byte
   range is calculated with respect to the encoded sequence of bytes,
   not the sequence of underlying bytes that would be obtained after
   decoding.

   Examples of bytes range specifiers:

   *  The first 500 bytes (byte offsets 0-499, inclusive):

           bytes=0-499

   *  The second 500 bytes (byte offsets 500-999, inclusive):

           bytes=500-999

   A client can limit the number of bytes requested without knowing the
   size of the selected representation.  If the last-pos value is
   absent, or if the value is greater than or equal to the current
   length of the representation data, the byte range is interpreted as
   the remainder of the representation (i.e., the server replaces the
   value of last-pos with a value that is one less than the current
   length of the selected representation).

   A client can refer to the last N bytes (N > 0) of the selected
   representation using a suffix-range.  If the selected representation
   is shorter than the specified suffix-length, the entire
   representation is used.

   Additional examples, assuming a representation of length 10000:

   *  The final 500 bytes (byte offsets 9500-9999, inclusive):

           bytes=-500

      Or:

           bytes=9500-

   *  The first and last bytes only (bytes 0 and 9999):

           bytes=0-0,-1

   *  The first, middle, and last 1000 bytes:

           bytes= 0-999, 4500-5499, -1000

   *  Other valid (but not canonical) specifications of the second 500
      bytes (byte offsets 500-999, inclusive):

           bytes=500-600,601-999
           bytes=500-700,601-999

   For a GET request, a valid bytes range-spec is satisfiable if it is
   either:

   *  an int-range with a first-pos that is less than the current length
      of the selected representation or

   *  a suffix-range with a non-zero suffix-length.

   When a selected representation has zero length, the only satisfiable
   form of range-spec in a GET request is a suffix-range with a non-zero
   suffix-length.

   In the byte-range syntax, first-pos, last-pos, and suffix-length are
   expressed as decimal number of octets.  Since there is no predefined
   limit to the length of content, recipients MUST anticipate
   potentially large decimal numerals and prevent parsing errors due to
   integer conversion overflows.
```

## 14.2.  Range (lines 6467-6542)

```
14.2.  Range

   The "Range" header field on a GET request modifies the method
   semantics to request transfer of only one or more subranges of the
   selected representation data (Section 8.1), rather than the entire
   selected representation.

     Range = ranges-specifier

   A server MAY ignore the Range header field.  However, origin servers
   and intermediate caches ought to support byte ranges when possible,
   since they support efficient recovery from partially failed transfers
   and partial retrieval of large representations.

   A server MUST ignore a Range header field received with a request
   method that is unrecognized or for which range handling is not
   defined.  For this specification, GET is the only method for which
   range handling is defined.

   An origin server MUST ignore a Range header field that contains a
   range unit it does not understand.  A proxy MAY discard a Range
   header field that contains a range unit it does not understand.

   A server that supports range requests MAY ignore or reject a Range
   header field that contains an invalid ranges-specifier
   (Section 14.1.1), a ranges-specifier with more than two overlapping
   ranges, or a set of many small ranges that are not listed in
   ascending order, since these are indications of either a broken
   client or a deliberate denial-of-service attack (Section 17.15).  A
   client SHOULD NOT request multiple ranges that are inherently less
   efficient to process and transfer than a single range that
   encompasses the same data.

   A server that supports range requests MAY ignore a Range header field
   when the selected representation has no content (i.e., the selected
   representation's data is of zero length).

   A client that is requesting multiple ranges SHOULD list those ranges
   in ascending order (the order in which they would typically be
   received in a complete representation) unless there is a specific
   need to request a later part earlier.  For example, a user agent
   processing a large representation with an internal catalog of parts
   might need to request later parts first, particularly if the
   representation consists of pages stored in reverse order and the user
   agent wishes to transfer one page at a time.

   The Range header field is evaluated after evaluating the precondition
   header fields defined in Section 13.1, and only if the result in
   absence of the Range header field would be a 200 (OK) response.  In
   other words, Range is ignored when a conditional GET would result in
   a 304 (Not Modified) response.

   The If-Range header field (Section 13.1.5) can be used as a
   precondition to applying the Range header field.

   If all of the preconditions are true, the server supports the Range
   header field for the target resource, the received Range field-value
   contains a valid ranges-specifier with a range-unit supported for
   that target resource, and that ranges-specifier is satisfiable with
   respect to the selected representation, the server SHOULD send a 206
   (Partial Content) response with content containing one or more
   partial representations that correspond to the satisfiable
   range-spec(s) requested.

   The above does not imply that a server will send all requested
   ranges.  In some cases, it may only be possible (or efficient) to
   send a portion of the requested ranges first, while expecting the
   client to re-request the remaining portions later if they are still
   desired (see Section 15.3.7).

   If all of the preconditions are true, the server supports the Range
   header field for the target resource, the received Range field-value
   contains a valid ranges-specifier, and either the range-unit is not
   supported for that target resource or the ranges-specifier is
   unsatisfiable with respect to the selected representation, the server
   SHOULD send a 416 (Range Not Satisfiable) response.
```

## 14.3.  Accept-Ranges (lines 6544-6584)

```
14.3.  Accept-Ranges

   The "Accept-Ranges" field in a response indicates whether an upstream
   server supports range requests for the target resource.

     Accept-Ranges     = acceptable-ranges
     acceptable-ranges = 1#range-unit

   For example, a server that supports byte-range requests
   (Section 14.1.2) can send the field

   Accept-Ranges: bytes

   to indicate that it supports byte range requests for that target
   resource, thereby encouraging its use by the client for future
   partial requests on the same request path.  Range units are defined
   in Section 14.1.

   A client MAY generate range requests regardless of having received an
   Accept-Ranges field.  The information only provides advice for the
   sake of improving performance and reducing unnecessary network
   transfers.

   Conversely, a client MUST NOT assume that receiving an Accept-Ranges
   field means that future range requests will return partial responses.
   The content might change, the server might only support range
   requests at certain times or under certain conditions, or a different
   intermediary might process the next request.

   A server that does not support any kind of range request for the
   target resource MAY send

   Accept-Ranges: none

   to advise the client not to attempt a range request on the same
   request path.  The range unit "none" is reserved for this purpose.

   The Accept-Ranges field MAY be sent in a trailer section, but is
   preferred to be sent as a header field because the information is
   particularly useful for restarting large information transfers that
   have failed in mid-content (before the trailer section is received).
```

## 14.4.  Content-Range (lines 6586-6671)

```
14.4.  Content-Range

   The "Content-Range" header field is sent in a single part 206
   (Partial Content) response to indicate the partial range of the
   selected representation enclosed as the message content, sent in each
   part of a multipart 206 response to indicate the range enclosed
   within each body part (Section 14.6), and sent in 416 (Range Not
   Satisfiable) responses to provide information about the selected
   representation.

     Content-Range       = range-unit SP
                           ( range-resp / unsatisfied-range )

     range-resp          = incl-range "/" ( complete-length / "*" )
     incl-range          = first-pos "-" last-pos
     unsatisfied-range   = "*/" complete-length

     complete-length     = 1*DIGIT

   If a 206 (Partial Content) response contains a Content-Range header
   field with a range unit (Section 14.1) that the recipient does not
   understand, the recipient MUST NOT attempt to recombine it with a
   stored representation.  A proxy that receives such a message SHOULD
   forward it downstream.

   Content-Range might also be sent as a request modifier to request a
   partial PUT, as described in Section 14.5, based on private
   agreements between client and origin server.  A server MUST ignore a
   Content-Range header field received in a request with a method for
   which Content-Range support is not defined.

   For byte ranges, a sender SHOULD indicate the complete length of the
   representation from which the range has been extracted, unless the
   complete length is unknown or difficult to determine.  An asterisk
   character ("*") in place of the complete-length indicates that the
   representation length was unknown when the header field was
   generated.

   The following example illustrates when the complete length of the
   selected representation is known by the sender to be 1234 bytes:

   Content-Range: bytes 42-1233/1234

   and this second example illustrates when the complete length is
   unknown:

   Content-Range: bytes 42-1233/*

   A Content-Range field value is invalid if it contains a range-resp
   that has a last-pos value less than its first-pos value, or a
   complete-length value less than or equal to its last-pos value.  The
   recipient of an invalid Content-Range MUST NOT attempt to recombine
   the received content with a stored representation.

   A server generating a 416 (Range Not Satisfiable) response to a byte-
   range request SHOULD send a Content-Range header field with an
   unsatisfied-range value, as in the following example:

   Content-Range: bytes */1234

   The complete-length in a 416 response indicates the current length of
   the selected representation.

   The Content-Range header field has no meaning for status codes that
   do not explicitly describe its semantic.  For this specification,
   only the 206 (Partial Content) and 416 (Range Not Satisfiable) status
   codes describe a meaning for Content-Range.

   The following are examples of Content-Range values in which the
   selected representation contains a total of 1234 bytes:

   *  The first 500 bytes:

      Content-Range: bytes 0-499/1234

   *  The second 500 bytes:

      Content-Range: bytes 500-999/1234

   *  All except for the first 500 bytes:

      Content-Range: bytes 500-1233/1234

   *  The last 500 bytes:

      Content-Range: bytes 734-1233/1234
```

## 14.6.  Media Type multipart/byteranges (lines 6697-6787)

```
14.6.  Media Type multipart/byteranges

   When a 206 (Partial Content) response message includes the content of
   multiple ranges, they are transmitted as body parts in a multipart
   message body ([RFC2046], Section 5.1) with the media type of
   "multipart/byteranges".

   The "multipart/byteranges" media type includes one or more body
   parts, each with its own Content-Type and Content-Range fields.  The
   required boundary parameter specifies the boundary string used to
   separate each body part.

   Implementation Notes:

   1.  Additional CRLFs might precede the first boundary string in the
       body.

   2.  Although [RFC2046] permits the boundary string to be quoted, some
       existing implementations handle a quoted boundary string
       incorrectly.

   3.  A number of clients and servers were coded to an early draft of
       the byteranges specification that used a media type of
       "multipart/x-byteranges", which is almost (but not quite)
       compatible with this type.

   Despite the name, the "multipart/byteranges" media type is not
   limited to byte ranges.  The following example uses an "exampleunit"
   range unit:

   HTTP/1.1 206 Partial Content
   Date: Tue, 14 Nov 1995 06:25:24 GMT
   Last-Modified: Tue, 14 July 04:58:08 GMT
   Content-Length: 2331785
   Content-Type: multipart/byteranges; boundary=THIS_STRING_SEPARATES

   --THIS_STRING_SEPARATES
   Content-Type: video/example
   Content-Range: exampleunit 1.2-4.3/25

   ...the first range...
   --THIS_STRING_SEPARATES
   Content-Type: video/example
   Content-Range: exampleunit 11.2-14.3/25

   ...the second range
   --THIS_STRING_SEPARATES--

   The following information serves as the registration form for the
   "multipart/byteranges" media type.

   Type name:  multipart

   Subtype name:  byteranges

   Required parameters:  boundary

   Optional parameters:  N/A

   Encoding considerations:  only "7bit", "8bit", or "binary" are
      permitted

   Security considerations:  see Section 17

   Interoperability considerations:  N/A

   Published specification:  RFC 9110 (see Section 14.6)

   Applications that use this media type:  HTTP components supporting
      multiple ranges in a single request

   Fragment identifier considerations:  N/A

   Additional information:  Deprecated alias names for this type:  N/A

                            Magic number(s):  N/A

                            File extension(s):  N/A

                            Macintosh file type code(s):  N/A

   Person and email address to contact for further information:  See Aut
      hors' Addresses section.

   Intended usage:  COMMON

   Restrictions on usage:  N/A

   Author:  See Authors' Addresses section.

   Change controller:  IESG
```

## 15.3.7.  206 Partial Content (lines 7066-7106)

```
15.3.7.  206 Partial Content

   The 206 (Partial Content) status code indicates that the server is
   successfully fulfilling a range request for the target resource by
   transferring one or more parts of the selected representation.

   A server that supports range requests (Section 14) will usually
   attempt to satisfy all of the requested ranges, since sending less
   data will likely result in another client request for the remainder.
   However, a server might want to send only a subset of the data
   requested for reasons of its own, such as temporary unavailability,
   cache efficiency, load balancing, etc.  Since a 206 response is self-
   descriptive, the client can still understand a response that only
   partially satisfies its range request.

   A client MUST inspect a 206 response's Content-Type and Content-Range
   field(s) to determine what parts are enclosed and whether additional
   requests are needed.

   A server that generates a 206 response MUST generate the following
   header fields, in addition to those required in the subsections
   below, if the field would have been sent in a 200 (OK) response to
   the same request: Date, Cache-Control, ETag, Expires,
   Content-Location, and Vary.

   A Content-Length header field present in a 206 response indicates the
   number of octets in the content of this message, which is usually not
   the complete length of the selected representation.  Each
   Content-Range header field includes information about the selected
   representation's complete length.

   A sender that generates a 206 response to a request with an If-Range
   header field SHOULD NOT generate other representation header fields
   beyond those required because the client already has a prior response
   containing those header fields.  Otherwise, a sender MUST generate
   all of the representation header fields that would have been sent in
   a 200 (OK) response to the same request.

   A 206 response is heuristically cacheable; i.e., unless otherwise
   indicated by explicit cache controls (see Section 4.2.2 of
   [CACHING]).
```

## 15.5.17.  416 Range Not Satisfiable (lines 7755-7785)

```
15.5.17.  416 Range Not Satisfiable

   The 416 (Range Not Satisfiable) status code indicates that the set of
   ranges in the request's Range header field (Section 14.2) has been
   rejected either because none of the requested ranges are satisfiable
   or because the client has requested an excessive number of small or
   overlapping ranges (a potential denial of service attack).

   Each range unit defines what is required for its own range sets to be
   satisfiable.  For example, Section 14.1.2 defines what makes a bytes
   range set satisfiable.

   A server that generates a 416 response to a byte-range request SHOULD
   generate a Content-Range header field specifying the current length
   of the selected representation (Section 14.4).

   For example:

   HTTP/1.1 416 Range Not Satisfiable
   Date: Fri, 20 Jan 2012 15:41:54 GMT
   Content-Range: bytes */47022

      |  *Note:* Because servers are free to ignore Range, many
      |  implementations will respond with the entire selected
      |  representation in a 200 (OK) response.  That is partly because
      |  most clients are prepared to receive a 200 (OK) to complete the
      |  task (albeit less efficiently) and partly because clients might
      |  not stop making an invalid range request until they have
      |  received a complete representation.  Thus, clients cannot
      |  depend on receiving a 416 (Range Not Satisfiable) response even
      |  when it is most appropriate.
```

## Facts this gives (derived, each tied to a block above)

* A server MAY ignore `Range` (the Range section) and answer `200` with the full representation; a client
  that sent `Range` must then handle a `200`. For HLS byte-range playlists that makes every I-frame fetch
  download the whole segment: correct but it defeats the purpose, so range support is required for the
  I-frame feature to be worth shipping (not for correctness).
* Only the `bytes` range unit is defined (§14.1.2); multiple ranges may be answered with
  `multipart/byteranges` (§14.6) or ignored with `200`.
* `206` carries `Content-Range: bytes first-last/complete-length` (§14.4, §15.3.7); an unsatisfiable range is
  `416` with `Content-Range: bytes */complete-length` (§15.5.17); `Accept-Ranges: bytes` advertises support (§14.3).
* `If-Range` (§13.1.5) makes a range conditional on a validator; segment resources here are immutable by
  name, so a stale validator is not a practical case, but the header must not be mishandled.
