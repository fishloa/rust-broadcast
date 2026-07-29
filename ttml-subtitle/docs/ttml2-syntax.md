# TTML2 element/attribute syntax reference

Source: **W3C Recommendation, "Timed Text Markup Language 2 (TTML2)", 08 November 2018**.
This version: `https://www.w3.org/TR/2018/REC-ttml2-20181108/`. Latest: `https://www.w3.org/TR/ttml2/`.
Fetched 2026-07-30 (see `README.md` in this directory for provenance).

All element/attribute syntax boxes below are transcribed **verbatim** from the
spec's own `<table class="syntax">` ("XML Representation" / "Syntax
Representation") boxes, keyed to the section number the box appears under in
the spec. Style-property summary rows (Values/Initial/Applies to/Inherited/
Percentages/Animatable) are transcribed verbatim from the spec's per-attribute
"common" property table. Section numbers are the spec's own numbering, taken
from the document's table of contents. Nothing in this file is inferred from
memory of TTML1/SMPTE-TT/other profiles — only from the fetched TTML2 REC text.

Grammar notation is the spec's own (informal EBNF: `:` = "is defined as",
`|` = alternation, `?`/`*`/`+` = optional/zero-or-more/one-or-more, `||` =
"one or more of, in any order", literals in `"..."`, non-terminals in
`<angle-brackets>`).

## 1. Namespaces (§5.1, Table 5-1)

| Name | Prefix | Value |
|---|---|---|
| TT | `tt:` | `http://www.w3.org/ns/ttml` |
| TT Parameter | `ttp:` | `http://www.w3.org/ns/ttml#parameter` |
| TT Style | `tts:` | `http://www.w3.org/ns/ttml#styling` |
| TT Audio Style | `tta:` | `http://www.w3.org/ns/ttml#audio` |
| TT Metadata | `ttm:` | `http://www.w3.org/ns/ttml#metadata` |
| TT Intermediate Synchronic Document | `isd:` | `http://www.w3.org/ns/ttml#isd` |
| TT Profile | (none) | `http://www.w3.org/ns/ttml/profile/` |
| TT Feature | (none) | `http://www.w3.org/ns/ttml/feature/` |
| TT Extension | (none) | `http://www.w3.org/ns/ttml/extension/` |
| TT Resource | (none) | `http://www.w3.org/ns/ttml/resource/` |

"TT Style Namespaces" (plural, used throughout the syntax boxes below to mean
"any attribute in `tts:` or `tta:`") = TT Style ∪ TT Audio Style.

All TTML namespaces are mutable: undefined names within them are reserved for
future W3C standardization — an implementation must not assume an unknown
name in these namespaces is safe to treat as a no-op extension.

## 2. Element vocabulary groups (§5.4.1, Table 5-3 / Table 5-4)

These are the content-model building blocks (`Foo.class`) referenced by the
`Content:` lines in the per-element syntax boxes in §3 below. Transcribed
verbatim from Table 5-4 – Element Vocabulary Groups:

| Group | Elements |
|---|---|
| `Animation.class` | `animate` \| `set` |
| `Block.class` | `div` \| `p` |
| `Data.class` | `data` |
| `Embedded.class` | `audio`, `image` |
| `Font.class` | `font` |
| `Inline.class` | `span` \| `br` \| `#PCDATA` |
| `Layout.class` | `region` |
| `Metadata.class` | `metadata` \| `ttm:agent` \| `ttm:copyright` \| `ttm:desc` \| `ttm:item` \| `ttm:title` |
| `Profile.class` | `ttp:profile` |

Table 5-3 – Element Vocabulary (module → elements, informative grouping used
by the spec's own catalog, not a content-model group):

| Module | Elements |
|---|---|
| Animation | `animate`, `animation`, `set` |
| Audio | `audio` |
| Content | `body`, `br`, `div`, `p`, `span` |
| Data | `chunk`, `data`, `resources`, `source` |
| Document | `tt` |
| Font | `font` |
| Head | `head` |
| Image | `image` |
| Layout | `layout`, `region` |
| Metadata | `metadata` |
| Metadata Items | `ttm:actor`, `ttm:agent`, `ttm:copyright`, `ttm:desc`, `ttm:item`, `ttm:name`, `ttm:title` |
| Profile | `ttp:features`, `ttp:feature`, `ttp:extensions`, `ttp:extension`, `ttp:profile` |
| Styling | `initial`, `styling`, `style` |

Note the difference between the two tables: `image` is in the `Embedded.class`
content-model group (with `audio`) but is catalogued as its own "Image" module
in Table 5-3; `font` belongs to no content-model group listed in Table 5-4
except via its own `Font.class` (used only inside `resources`, see §7 below).

## 3. Element syntax (verbatim XML Representation boxes)

Each box below is the spec's literal `<Element ... Content: ...>` syntax
table, tagged with the section it is defined in. `IDREF`/`IDREFS`/`ID`/
`xsd:*` are the corresponding XML Schema / XML `id` types.

### 3.1 Profile vocabulary (§6 Profile)

`ttp:profile` (§6.1.1):
```
<ttp:profile
  combine = ("leastRestrictive" | "mostRestrictive" | "replace") : replace
  designator = xsd:string
  type = ("processor" | "content") : processor
  use = xsd:string
  xml:base = <uri>
  xml:id = ID
  Content: Metadata.class*, ((ttp:features*, ttp:extensions*)|ttp:profile*)
</ttp:profile>
```

`ttp:features` (§6.1.2):
```
<ttp:features
  xml:base = <uri> : TT Feature Namespace
  xml:id = ID
  Content: Metadata.class*, ttp:feature*
</ttp:features>
```

`ttp:feature` (§6.1.3):
```
<ttp:feature
  extends = xsd:string
  restricts = xsd:string
  value = ("optional" | "required" | "use" | "prohibited") : see prose
  xml:id = ID
  Content: #PCDATA
</ttp:feature>
```

`ttp:extensions` (§6.1.4):
```
<ttp:extensions
  xml:base = <uri> : TT Extension Namespace
  xml:id = ID
  Content: Metadata.class*, ttp:extension*
</ttp:extensions>
```

`ttp:extension` (§6.1.5):
```
<ttp:extension
  extends = xsd:string
  restricts = xsd:string
  value = ("optional" | "required" | "use" | "prohibited") : see prose
  xml:id = ID
  Content: #PCDATA
</ttp:extension>
```

Profile *attributes* on the root `tt` element (§6.2, each is a global
attribute, not an element): `ttp:contentProfiles`, `ttp:contentProfileCombination`,
`ttp:inferProcessorProfileMethod`, `ttp:inferProcessorProfileSource`,
`ttp:permitFeatureNarrowing`, `ttp:permitFeatureWidening`, `ttp:profile`,
`ttp:processorProfiles`, `ttp:processorProfileCombination`, `ttp:validation`,
`ttp:validationAction`. Value grammars (§6.2.1–6.2.11):

```
ttp:contentProfiles          : designators | "all(" <lwsp> designators <lwsp> ")"
ttp:contentProfileCombination: "leastRestrictive" | "mostRestrictive" | "replace" | "ignore"
ttp:inferProcessorProfileMethod: "loose" | "strict"
ttp:inferProcessorProfileSource: "combined" | "first"
ttp:permitFeatureNarrowing    : xsd:boolean
ttp:permitFeatureWidening     : xsd:boolean
ttp:profile (attribute)       : designator   (designator: <profile-designator>)
ttp:processorProfiles         : designators | "all(" <lwsp> designators <lwsp> ")" | "any(" <lwsp> designators <lwsp> ")"
ttp:processorProfileCombination: "leastRestrictive" | "mostRestrictive" | "replace" | "ignore"
ttp:validation                : "required" | "optional" | "prohibited"
ttp:validationAction          : "abort" | "warn" | "ignore"
designators                   : designator (<lwsp> designator)*
designator                    : <profile-designator>
```

#### Profile designators (§5.2.3, Table 5-2)

A profile is referenced by an `<absolute-profile-designator>` (external),
`<relative-profile-designator>` (external, resolved against the TT Profile
Namespace as base URI), or `<fragment-profile-designator>` (internal/inline).
All must adhere to `<uri>` syntax. **Any designator with the TT Profile
Namespace (`http://www.w3.org/ns/ttml/profile/`) as prefix that is not one of
the standard designators below is reserved for future standardization and
must not appear in a conformant document instance** — a parser encountering
one should treat it as invalid, not silently accept it.

Table 5-2 – Profiles (standard designators):

| Name | Absolute Designator |
|---|---|
| DFXP Full | `http://www.w3.org/ns/ttml/profile/dfxp-full` |
| DFXP Presentation | `http://www.w3.org/ns/ttml/profile/dfxp-presentation` |
| DFXP Transformation | `http://www.w3.org/ns/ttml/profile/dfxp-transformation` |
| SDP US | `http://www.w3.org/ns/ttml/profile/sdp-us` |
| TTML2 Full | `http://www.w3.org/ns/ttml/profile/ttml2-full` |
| TTML2 Presentation | `http://www.w3.org/ns/ttml/profile/ttml2-presentation` |
| TTML2 Transformation | `http://www.w3.org/ns/ttml/profile/ttml2-transformation` |

Content profile: declares constraints the *document* adheres to (via
`ttp:contentProfiles` attribute and/or `ttp:profile` elements of
`type="content"`). Processor profile: declares what a *processor* must
support (`ttp:processorProfiles` attribute, `ttp:profile` elements of
`type="processor"`, or the `ttp:profile` attribute). A predefined profile can
be supersetted (some feature/extension promoted to required that was
optional/absent) or subsetted (some required feature/extension demoted to
optional) — the combination is additive, subtractive, or both.

### 3.2 Parameter vocabulary (§7 Parameter — attributes only, no elements)

All parameter attributes are **significant only when specified on the `tt`
root element** (stated explicitly for each one in the spec). Value grammars
(§7.2.1–7.2.11), each verbatim:

```
ttp:cellResolution     : columns <lwsp> rows              // columns != 0; rows != 0
                          columns | rows : <digit>+
ttp:clockMode          : "local" | "gps" | "utc"
ttp:displayAspectRatio : numerator <lwsp> denominator      // numerator != 0; denominator != 0
                          numerator | denominator : <digit>+
ttp:dropMode           : "dropNTSC" | "dropPAL" | "nonDrop"
ttp:frameRate          : <digit>+                          // value > 0
ttp:frameRateMultiplier: numerator <lwsp> denominator      // numerator != 0; denominator != 0
ttp:markerMode         : "continuous" | "discontinuous"
ttp:pixelAspectRatio   : numerator <lwsp> denominator      // numerator != 0; denominator != 0
ttp:subFrameRate       : <digit>+                          // value > 0
ttp:tickRate           : <digit>+                          // value > 0
ttp:timeBase           : "media" | "smpte" | "clock"
```

Defaults / semantics (§7.2, prose, verbatim meaning preserved):

- `ttp:cellResolution`: default **32 columns × 15 rows** if absent (max CTA-608-E grid). Used only for measuring lengths in `c` (cell) units and coordinates — not implied glyph-grid alignment.
- `ttp:clockMode`: default **`utc`**. Only meaningful when `ttp:timeBase="clock"`. `local`=wall-clock, `utc`=UTC, `gps`=GPS time (not leap-second adjusted, unlike UTC).
- `ttp:displayAspectRatio`: no default value stated here — see Appendix H (Root Container Region Semantics) for the fallback when absent.
- `ttp:dropMode`: default **`nonDrop`**. Only meaningful when `ttp:timeBase="smpte"`. `nonDrop`: frames count 0..N-1 every second (N = frame rate), ignoring `ttp:frameRateMultiplier`. `dropNTSC`: frame codes 00,01 dropped at the start of every minute except multiples of 10. `dropPAL`: frame codes 00-03 dropped at the start of every *even* minute except multiples of 20 (used for M/PAL, Brazil).
- `ttp:frameRate`: default **30 fps** if absent and no application default applies. Must be > 0.
- `ttp:frameRateMultiplier`: default **1:1** if absent. Nominal NTSC multiplier is 1000:1001 (yields ≈29.97 fps from a nominal 30fps rate).
- `ttp:markerMode`: default **`discontinuous`**. Only meaningful when `ttp:timeBase="smpte"`. `continuous`: SMPTE time coordinates are linear/monotonic and may be converted to real time. `discontinuous`: no such assumption; arithmetic on time expressions (including any duration) is undefined/invalid.
- `ttp:pixelAspectRatio`: should not be specified unless `tts:extent` on `tt` uses `px` units on both components; otherwise deprecated.
- `ttp:subFrameRate`: default **1** if absent. Only meaningful with frame-based time expressions that include a sub-frame component.
- `ttp:tickRate`: default, if a frame rate is specified, **effective frame rate × sub-frame rate**; else **1 tick/second**. Must not be 0.
- `ttp:timeBase`: default **`media`** if absent. `media`: a coordinate on some media object's (or the document's own) timeline. `smpte`: a SMPTE ST 12-1 time coordinate (`ttp:markerMode`/`ttp:dropMode` apply). `clock`: a real-world time coordinate.

### 3.3 Content vocabulary (§8 Content)

`tt` (root, §8.1.1):
```
<tt
  tts:extent = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve") : default
  {any attributes in TT Parameter Namespace}
  Content: head?, body?
</tt>
```

`head` (§8.1.2):
```
<head
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, Profile.class*, resources?, styling?, layout?, animation?
</head>
```

`body` (§8.1.3):
```
<body
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  region = IDREF
  style = IDREFS
  timeContainer = ("par" | "seq")
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*, div*
</body>
```

`div` (§8.1.4):
```
<div
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  region = IDREF
  style = IDREFS
  timeContainer = ("par" | "seq")
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*, Layout.class?, (Block.class|Embedded.class)*
</div>
```

`p` (§8.1.5):
```
<p
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  region = IDREF
  style = IDREFS
  timeContainer = ("par" | "seq")
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*, Layout.class?, (Inline.class|Embedded.class)*
</p>
```
If no `timeContainer` is given, `p` is a **parallel** ("par") time container.
A run of children consisting solely of character data ("#PCDATA") is an
*anonymous span* for style-inheritance purposes (§12.4.1).

`span` (§8.1.6):
```
<span
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  region = IDREF
  style = IDREFS
  timeContainer = ("par" | "seq")
  xlink:arcrole = <uri-list>
  xlink:href = <uri>
  xlink:role = <uri-list>
  xlink:show = ("new" | "replace" | "embed" | "other" | "none") : new
  xlink:title = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*, (Inline.class|Embedded.class)*
</span>
```

`br` (§8.1.7):
```
<br
  condition = <condition>
  style = IDREFS
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*
</br>
```
Note: `br` has no `begin`/`end`/`dur`/`timeContainer` — it is not itself
timed; its implicit duration is defined equal to an anonymous span (§12.4).

Content value expressions (§8.3, verbatim grammars) — condition/expression
mini-language used by the `condition` attribute (present on every content
element above) and by `<supports-function>`/`<media-function>`/
`<parameter-function>`:

```
<condition>            : <expression>
<expression>           : logical-or-expression
logical-or-expression  : logical-and-expression | logical-or-expression <lwsp>? "||" <lwsp>? logical-and-expression
logical-and-expression : equality-expression | logical-and-expression <lwsp>? "&&" <lwsp>? equality-expression
equality-expression    : relational-expression | equality-expression <lwsp>? ("=="|"!=") <lwsp>? relational-expression
relational-expression  : additive-expression | relational-expression <lwsp>? ("<"|">"|"<="|">=") <lwsp>? additive-expression
additive-expression    : multiplicitive-expression | additive-expression <lwsp>? ("+"|"-") <lwsp>? multiplicitive-expression
multiplicitive-expression: unary-expression | multiplicitive-expression <lwsp>? ("*"|"/"|"%") <lwsp>? unary-expression
unary-expression       : primary-or-function-expression | ("+"|"-"|"!") <lwsp>? unary-expression
primary-or-function-expression: primary-expression | function-expression
primary-expression     : identifier | literal | "(" <lwsp>? expression <lwsp>? ")"
function-expression    : identifier <arguments>
identifier              : xsd:NCName
literal                 : boolean-literal | numeric-literal | string-literal
boolean-literal         : "true" | "false"
numeric-literal         : decimal-literal  // standard JS-like decimal grammar
string-literal          : <quoted-string>

<condition-function>   : <media-function> | <parameter-function> | <supports-function>
<media-function>       : "media(" <lwsp>? media-query <lwsp>? ")"     // media-query: <quoted-string>
<parameter-function>   : "parameter(" <lwsp>? parameter-name <lwsp>? ")"  // parameter-name: <quoted-string>
<supports-function>     : "supports(" <lwsp>? feature-or-extension-designator <lwsp>? ")"  // : <quoted-string>
<bound-parameter>      : "forced" | "mediaAspectRatio" | "mediaLanguage" | "userLanguage"

<uri>                  : xsd:anyURI
<uri-list>              : <uri> (<lwsp> <uri>)*
<absolute-uri>          : <uri>   // absolute form only
<relative-uri>          : <uri>   // no scheme component present
<fragment-uri>          : <uri>   // fragment component only
<profile-designator>   : <absolute-profile-designator> | <relative-profile-designator> | <fragment-profile-designator>
<quoted-string>         : '"' ([^"\\]|escape)* '"'  |  "'" ([^'\\]|escape)* "'"   // escape: '\' char
```

### 3.4 Embedded content vocabulary (§9 Embedded Content)

`audio` (§9.1.1):
```
<audio
  animate = IDREFS
  begin = <time-expression>
  clipBegin = <time-expression>
  clipEnd = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  src = <audio>
  style = IDREFS
  timeContainer = ("par" | "seq")
  type = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  {any attributes in TT Metadata Namespace}
  Content: Metadata.class*, Animation.class*, source*
</audio>
```

`chunk` (§9.1.2, base64-encoded fragment of an embedded `data` resource):
```
<chunk
  condition = <condition>
  encoding = ("base16" | "base32" | "base32hex" | "base64" | "base64url") : base64
  length = xsd:nonNegativeInteger
  xml:base = <uri>
  xml:id = ID
  Content: #PCDATA
</chunk>
```

`data` (§9.1.3, an embedded binary resource, inline or chunked, or by
reference via `source`):
```
<data
  condition = <condition>
  encoding = ("base16" | "base32" | "base32hex" | "base64" | "base64url") : see prose
  format = <data-format>
  length = xsd:nonNegativeInteger
  src = <data>
  type = xsd:string : see prose
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: #PCDATA | (Metadata.class*, chunk+) | (Metadata.class*, source+)
</data>
```

`font` (§9.1.4, only valid inside `resources`, see Font.class group):
```
<font
  condition = <condition>
  family = xsd:string
  range = <unicode-range>
  style = ("normal" | "italic" | "oblique") : see prose
  src = <font>
  type = xsd:string
  weight = ("normal" | "bold") : see prose
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, source*
</font>
```

`image` (§9.1.5):
```
<image
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  region = IDREF
  src = <image>
  style = IDREFS
  timeContainer = ("par" | "seq")
  type = xsd:string
  xlink:arcrole = <uri-list>
  xlink:href = <uri>
  xlink:role = <uri-list>
  xlink:show = ("new" | "replace" | "embed" | "other" | "none") : new
  xlink:title = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  {any attributes in TT Metadata Namespace}
  Content: Metadata.class*, Animation.class*, source*
</image>
```

`resources` (§9.1.6, top-level container inside `head`):
```
<resources
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, (Data.class|Embedded.class|Font.class)*
</resources>
```

`source` (§9.1.7, reference to an out-of-line or alternative resource):
```
<source
  condition = <condition>
  format = <data-format>
  src = <data>
  type = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, data?
</source>
```

Embedded-content value expressions (§9.3):
```
<audio>          : <uri>
<data>           : <uri>
<data-format>    : xsd:NCName | <uri>
<font>           : <uri>
<image>          : <uri>
<unicode-range>  : range (<lwsp>? "," <lwsp>? range)*
range            : codepoint | codepoint "-" codepoint
codepoint        : ("U"|"u") "+" hexdigit-or-wildcard{1,6}
```

### 3.5 Styling vocabulary (§10 Styling)

`initial` (§10.1.1 — sets a document-wide default value for a style
property, as an attribute on this element):
```
<initial
  condition = <condition>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*
</initial>
```

`style` (§10.1.2 — a referable, chainable style set):
```
<style
  condition = <condition>
  style = IDREFS
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*
</style>
```

`styling` (§10.1.3 — top-level container inside `head`):
```
<styling
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, initial*, style*
</styling>
```

#### Style attribute property table (§10.2.2–10.2.56, verbatim `Values`/
`Initial`/`Applies to`/`Inherited`/`Percentages`/`Animatable` rows)

All are attributes in the `tts:` namespace unless noted `tta:` (TT Audio
Style). "Applies to" lists which element(s) the property has defined
semantics on — an implementation should not fabricate meaning for a style
property present on an element outside this list. `style` itself
(`IDREFS`, §10.2.1, the style-reference binding attribute) has no such
property table since it is not a style property, only a binding.

| Property | Values | Initial | Applies to | Inherited | Animatable |
|---|---|---|---|---|---|
| `tts:backgroundClip` | `"border"⎮"content"⎮"padding"` | `border` | body, div, image, p, region, span | no | discrete |
| `tts:backgroundColor` | `<color>` | `transparent` | body, div, image, p, region, span | no | discrete; continuous |
| `tts:backgroundExtent` | `<extent>` | `auto` | body, div, image, p, region, span | no | discrete |
| `tts:backgroundImage` | `"none"⎮<image>` | `none` | body, div, image, p, region, span | no | discrete |
| `tts:backgroundOrigin` | `"border"⎮"content"⎮"padding"` | `padding` | body, div, image, p, region, span | no | discrete |
| `tts:backgroundPosition` | `<position>` | `0% 0%` | body, div, image, p, region, span | no | discrete |
| `tts:backgroundRepeat` | `"repeat"⎮"repeatX"⎮"repeatY"⎮"noRepeat"` | `repeat` | body, div, image, p, region, span | no | discrete |
| `tts:border` | `<border>` | `none` | body, div, image, p, region, span | no | discrete; continuous (color only) |
| `tts:bpd` | `<measure>` | `auto` | body, div, p, span | no | discrete |
| `tts:color` | `<color>` | see prose | span | yes | discrete; continuous |
| `tts:direction` | `"ltr"⎮"rtl"` | `ltr` | p, span | yes, but see special semantics | discrete |
| `tts:disparity` | `<length>` | `0px` | region; see special usage for div and p | no | discrete; continuous |
| `tts:display` | `"auto"⎮"none"⎮"inlineBlock"` | `auto` | body, div, image, p, region, span | no | discrete |
| `tts:displayAlign` | `"before"⎮"center"⎮"after"⎮"justify"` | `before` | body, div, p, region | no | discrete |
| `tts:extent` | `<extent>` | `auto` | tt, region, image; see special usage for div and p | no | discrete |
| `tts:fontFamily` | `<font-families>` | `default` | span | yes | discrete |
| `tts:fontKerning` | `"none"⎮"normal"` | `normal` | span | yes | discrete |
| `tts:fontSelectionStrategy` | `"auto"⎮"character"` | `auto` | span | yes | discrete |
| `tts:fontShear` | `<percentage>` | `0%` | span | yes | discrete |
| `tts:fontSize` | `<font-size>` | `1c` | span | yes, excepting §10.2.21.1 special semantics | discrete |
| `tts:fontStyle` | `"normal"⎮"italic"⎮"oblique"` | `normal` | span | yes | discrete |
| `tts:fontVariant` | `<font-variant>` | `normal` | span | yes | discrete |
| `tts:fontWeight` | `"normal"⎮"bold"` | `normal` | span | yes | discrete |
| `tts:ipd` | `<measure>` | `auto` | body, div, p, span | no | discrete |
| `tts:letterSpacing` | `"normal"⎮<length>` | `normal` | span | yes | discrete |
| `tts:lineHeight` | `"normal"⎮<length>` | `normal` | p | yes, but see special semantics | discrete |
| `tts:lineShear` | `<percentage>` | `0%` | p | yes | discrete |
| `tts:luminanceGain` | `<non-negative-number>` | `1.0` | region | no | discrete; continuous |
| `tts:opacity` | `<alpha>` | `1.0` | body, div, image, p, region, span | no | discrete; continuous |
| `tts:origin` | `<origin>` | `auto` | region; see special usage for div and p | no | discrete |
| `tts:overflow` | `"visible"⎮"hidden"` | `hidden` | region | no | discrete |
| `tts:padding` | `<padding>` | `0px` | body, div, image, p, region, span | no | discrete |
| `tts:position` | `<position>` | `top left` | region; see special usage for div and p | no | discrete; continuous |
| `tts:ruby` | `"none"⎮"container"⎮"base"⎮"baseContainer"⎮"text"⎮"textContainer"⎮"delimiter"` | `none` | span | no | none |
| `tts:rubyAlign` | `"start"⎮"center"⎮"end"⎮"spaceAround"⎮"spaceBetween"⎮"withBase"` | `center` | span, only if computed `tts:ruby` is `container` | yes | discrete |
| `tts:rubyPosition` | `<annotation-position>` | `outside` | span, only if computed `tts:ruby` is `textContainer` or `text` | yes | discrete |
| `tts:rubyReserve` | `<ruby-reserve>` | `none` | p | yes | discrete |
| `tts:shear` | `<percentage>` | `0%` | p | yes | discrete |
| `tts:showBackground` | `"always"⎮"whenActive"` | `always` | region | no | discrete |
| `tts:textAlign` | `"left"⎮"center"⎮"right"⎮"start"⎮"end"⎮"justify"` | `start` | p | see prose | discrete |
| `tts:textCombine` | `<text-combine>` | `none` | span | yes | discrete |
| `tts:textDecoration` | `<text-decoration>` | `none` | span | yes, but see special semantics | discrete |
| `tts:textEmphasis` | `<text-emphasis>` | `none` | span | yes | discrete; continuous (color only) |
| `tts:textOrientation` | `"mixed"⎮"sideways"⎮"upright"` | `mixed` | span | yes | discrete |
| `tts:textOutline` | `<text-outline>` | `none` | span | yes | discrete; continuous (color only) |
| `tts:textShadow` | `<text-shadow>` | `none` | span | yes | discrete; continuous (color only) |
| `tts:unicodeBidi` | `"normal"⎮"embed"⎮"bidiOverride"⎮"isolate"` | `normal` | p, span | no | discrete |
| `tts:visibility` | `"visible"⎮"hidden"` | `visible` | body, div, image, p, region, span | yes | discrete |
| `tts:wrapOption` | `"wrap"⎮"noWrap"` | `wrap` | span | yes | discrete |
| `tts:writingMode` | `"lrtb"⎮"rltb"⎮"tbrl"⎮"tblr"⎮"lr"⎮"rl"⎮"tb"` | `lrtb` | region | no | discrete |
| `tts:zIndex` | `"auto"⎮<integer>` | `auto` | region | no | discrete |
| `tta:gain` | `<number>` | `1` | audio, body, div, p, span | no | discrete; continuous |
| `tta:pan` | `<number>` | `0` | audio, body, div, p, span | no | discrete; continuous |
| `tta:pitch` | `<pitch>` | `0%` | span | yes | none |
| `tta:speak` | `"none"⎮"normal"⎮"fast"⎮"slow"` | `none` | span | yes | none |

#### Styling value expressions (§10.3, verbatim grammars — the value types
referenced by the `Values` column above)

```
<alpha>              : float
<annotation-color>   : "current" | <color>
<annotation-position>: "before" | "after" | "outside"
<border>             : <border-thickness> || <border-style> || <border-color> || <border-radii>
<border-color>       : <color>
<border-radii>       : "radii(" <lwsp>? <length> (<lwsp>? "," <lwsp>? <length>)? <lwsp>? ")"
<border-style>       : "none" | "dotted" | "dashed" | "solid" | "double"
<border-thickness>   : "thin" | "medium" | "thick" | <length>
<color>              : "#" rrggbb | "#" rrggbbaa | "rgb(" r,g,b ")" | "rgba(" r,g,b,a ")" | <named-color>
                        rrggbb: <hex-digit>{6}; rrggbbaa: <hex-digit>{8}; each component: <non-negative-integer> in [0,255]
<named-color>        : transparent(#00000000) | black(#000000ff) | silver(#c0c0c0ff) | gray(#808080ff)
                        | white(#ffffffff) | maroon(#800000ff) | red(#ff0000ff) | purple(#800080ff)
                        | fuchsia(#ff00ffff) | magenta(=fuchsia) | green(#008000ff) | lime(#00ff00ff)
                        | olive(#808000ff) | yellow(#ffff00ff) | navy(#000080ff) | blue(#0000ffff)
                        | teal(#008080ff) | aqua(#00ffffff) | cyan(=aqua)
<digit>              : "0".."9"
<emphasis-color>     : <annotation-color>
<emphasis-position>  : <annotation-position>
<emphasis-style>     : "none" | "auto" | ("filled"|"open") || ("circle"|"dot"|"sesame") | <quoted-string>
<extent>             : "auto" | "contain" | "cover" | <measure> <lwsp> <measure>
<family-name>         : unquoted-string | <quoted-string>
<font-families>       : font-family (<lwsp>? "," <lwsp>? font-family)*   // font-family: <family-name>|<generic-family-name>
<font-size>           : <length> (<lwsp> <length>)?    // 1 value = both dims; 2 = horizontal, vertical
<font-variant>        : "normal" | ("super"|"sub") || ("full"|"half") || "ruby"
<generic-family-name>: "default"|"monospace"|"sansSerif"|"serif"|"monospaceSansSerif"|"monospaceSerif"|"proportionalSansSerif"|"proportionalSerif"
<hex-digit>          : <digit> | "a".."f" | "A".."F"
<integer>            : ("+"|"-")? <non-negative-integer>
<length>             : scalar | <percentage>       // scalar: <number> units; units: "px"|"em"|"c"(cell)|"rw"|"rh"
<lwsp>                : <whitespace>+
<measure>            : "auto" | "fitContent" | "maxContent" | "minContent" | <length>
<non-negative-integer>: <digit>+
<non-negative-number> : <non-negative-integer> | non-negative-real   // non-negative-real: <digit>*"."<digit>+
<number>             : sign? <non-negative-number>
<origin>             : "auto" | <length> <lwsp> <length>
<padding>            : 1, 2, 3, or 4 <length> values (CSS shorthand order)
<percentage>          : <number> "%"
<pitch>              : <percentage> | <number> pitch-units?    // pitch-units: "hz"|"st"
<position>           : CSS2-style 1..4-component position grammar (edge keywords "left"/"right"/"top"/"bottom"/"center" + <length> offsets)
<ruby-reserve>       : "none" | ("both"|<annotation-position>) (<lwsp> <length>)?
<shadow>             : <length> <lwsp> <length> (<lwsp> <color>)? | <length> <lwsp> <length> <lwsp> <length> (<lwsp> <color>)?
<text-combine>       : "none" | "all"
<text-decoration>    : "none" | (("underline"|"noUnderline") || ("lineThrough"|"noLineThrough") || ("overline"|"noOverline"))
<text-emphasis>      : <emphasis-style> || <emphasis-color> || <emphasis-position>
<text-outline>       : "none" | (<color> <lwsp>)? <length> (<lwsp> <length>)?
<text-shadow>        : "none" | <shadow> (<lwsp>? "," <lwsp>? <shadow>)*
<whitespace>          : " " | "\t" | "\n" | "\r"
```
(`<border>`'s `||` combinator means each of the four components may appear at
most once, in any order, and each is independently optional — i.e. any
non-empty subset in any order.)

### 3.6 Layout vocabulary (§11 Layout)

`layout` (§11.1.1 — top-level container inside `head`):
```
<layout
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, region*
</layout>
```

`region` (§11.1.2):
```
<region
  animate = IDREFS
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  style = IDREFS
  timeContainer = ("par" | "seq")
  ttm:role = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*, Animation.class*, style*
</region>
```
Note `region` has no `region` attribute of its own (it cannot flow into
another region) but content elements bind to it via their own `region=IDREF`
attribute (§11.2.1). Its implicit duration is indefinite (§12.4).

### 3.7 Timing vocabulary (§12 Timing)

No timing *elements* are defined (§12.1) — only attributes. `begin`, `dur`,
`end` are unprefixed (not namespace-qualified) attributes shared across the
timed element types: `audio`, `body`, `div`, `image`, `p`, `region`, `set`
(via its own box), `span` and `animate` (via its own box). `timeContainer`
applies only to: `audio`, `body`, `div`, `image`, `p`, `region`, `span`
(§12.2.4, explicit list).

- **`begin`** (§12.2.1): the interval's begin point, included in the interval
  (left-closed). Default (no `begin` specified) is `0s`, from the nearest
  time-container ancestor. Must be a `<time-expression>` (§12.3.1) if given.
- **`dur`** (§12.2.2): the interval's duration. May legally be `0s` (a
  deliberate divergence from SMIL 3.0 §5.4.3, which this attribute otherwise
  follows). If `ttp:timeBase="smpte"` and `ttp:markerMode="discontinuous"`, a
  well-formed `dur` **must not** be specified on any element. If both `end`
  and `dur` are given, the active duration is the *lesser* of `dur` and
  (`end` − begin time).
- **`end`** (§12.2.3): the interval's ending point, excluded from the
  interval (right-open). Presentation includes the frame/tick immediately
  before the boundary, not the boundary frame itself.
- **`timeContainer`** (§12.2.4): `"par"` (default if absent) — children's
  intervals apply simultaneously, each relative to the container's own
  interval, with default SMIL `endsync="all"` (note: TTML's default of `all`
  differs from SMIL 3.0's own default of `last`). `"seq"` — children's
  intervals apply in sequence, each relative to the *previous sibling's*
  interval (or the container's own interval for the first child). Each time
  container is an independent time base/coordinate system.

Implicit durations (§12.4, verbatim rules):
- Anonymous span: indefinite if parent is a `par` container, else zero (`seq`).
- `animate`, `audio`, `br`, `image`, `set`: same as an anonymous span.
- `span` with non-mixed content (`#PCDATA` only): same as an anonymous span.
- `body`/`div`/`p`/`span` with mixed content: per SMIL endsync rules for the container type.
- `region`: indefinite.

#### `<time-expression>` (§12.3.1, verbatim grammar)

```
<time-expression> : clock-time | offset-time | wallclock-time

clock-time    : hours ":" minutes ":" seconds ( fraction | ":" frames ("." sub-frames)? )?
offset-time   : time-count fraction? metric
wallclock-time: "wallclock(" <lwsp>? ( date-time | wall-time | date ) <lwsp>? ")"

date-time     : date "T" wall-time
wall-time     : hhmm-time | hhmmss-time
date          : years "-" months "-" days
hhmm-time     : hours2 ":" minutes
hhmmss-time   : hours2 ":" minutes ":" seconds fraction?

years                                      : <digit><digit><digit><digit>
hours                                      : hours2 | hours3plus
hours3plus                                 : <digit><digit><digit>+
months | days | hours2 | minutes | seconds : <digit><digit>
frames                                     : <digit><digit> | <digit><digit><digit>+
sub-frames                                 : <digit>+
fraction                                   : "." <digit>+
time-count                                 : <digit>+

metric : "h" (hours) | "m" (minutes) | "s" (seconds) | "ms" (milliseconds) | "f" (frames) | "t" (ticks)
```

Constraints (verbatim, §12.3.1 prose):
- No `<whitespace>` anywhere in a `<time-expression>` except where `<lwsp>` is explicitly permitted (inside `wallclock(...)`).
- Clock-time: leading zeroes required for hours/minutes/seconds/frames < 10.
- Minutes ∈ [0,59]. Seconds ∈ [0,60] closed interval — 60 denotes a leap second, and is deprecated except when `ttp:timeBase="clock"` and `ttp:clockMode` is `local` or `utc`; elsewhere a value of 60 must be interpreted as if 59 had been specified.
- A `frames` term (or `f` metric) is an **error** when the clock time base applies. When present with a clock-time, its value must be in `[0, F-1]` where F = `ttp:frameRate`.
- A `sub-frames` term is an **error** when the clock time base applies. Value must be in `[0, S-1]` where S = `ttp:subFrameRate`.
- `wallclock-time` is an **error** if the governing time base is not `clock`.
- If the governing time base is `smpte`: `offset-time` form is deprecated, and a fractional-seconds component in a `clock-time` is deprecated.
- No time-zone information is representable — convert to UTC and use `ttp:clockMode="utc"` instead.

### 3.8 Animation vocabulary (§13 Animation)

`animate` (§13.1.1):
```
<animate
  begin = <time-expression>
  calcMode = <calculation-mode>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  fill = <fill>
  keySplines = <key-splines>
  keyTimes = <key-times>
  repeatCount = <repeat-count>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*
</animate>
```

`animation` (§13.1.2 — grouping wrapper for out-of-line `animate`/`set`):
```
<animation
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: Metadata.class*, Animation.class*
</animation>
```

`set` (§13.1.3):
```
<set
  begin = <time-expression>
  condition = <condition>
  dur = <time-expression>
  end = <time-expression>
  fill = <fill>
  repeatCount = <repeat-count>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Style Namespaces}
  Content: Metadata.class*
</set>
```

Animation value expressions (§13.3, verbatim):
```
<animation-value>      : ([^;]|escape)+
<animation-value-list> : <animation-value> (<lwsp>? ";" <lwsp>? <animation-value>)+
<calculation-mode>     : "discrete" | "linear" | "paced" | "spline"
<fill>                 : "freeze" | "remove"
<key-splines>          : control (<lwsp>? ";" <lwsp>? control)*    // control: x1 y1 x2 y2, each a coordinate in [0,1]
<key-times>            : time (<lwsp>? ";" <lwsp>? time)*          // time: value in [0,1]
<repeat-count>         : count | "indefinite"     // count > 0, may be fractional
```

### 3.9 Metadata vocabulary (§14 Metadata)

`metadata` (§14.1.1):
```
<metadata
  condition = <condition>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  {any attributes in TT Metadata Namespace}
  Content: (Data.class|{any element in TT Metadata Namespace})*
</metadata>
```

`ttm:actor` (§14.1.2, empty, binds to a `ttm:agent` via `agent=IDREF`):
```
<ttm:actor
  agent = IDREF
  condition = <condition>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: EMPTY
</ttm:actor>
```

`ttm:agent` (§14.1.3):
```
<ttm:agent
  condition = <condition>
  type = ("person" | "character" | "group" | "organization" | "other")
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: ttm:name*, ttm:actor?
</ttm:agent>
```

`ttm:copyright`, `ttm:desc`, `ttm:title` (§14.1.4, §14.1.5, §14.1.8 — all
`#PCDATA`-only, identical attribute set):
```
<ttm:copyright  condition=<condition> xml:base=<uri> xml:id=ID xml:lang=xsd:string xml:space=("default"|"preserve") Content: #PCDATA>
<ttm:desc       condition=<condition> xml:base=<uri> xml:id=ID xml:lang=xsd:string xml:space=("default"|"preserve") Content: #PCDATA>
<ttm:title      condition=<condition> xml:base=<uri> xml:id=ID xml:lang=xsd:string xml:space=("default"|"preserve") Content: #PCDATA>
```

`ttm:item` (§14.1.6, nestable named-value pair):
```
<ttm:item
  condition = <condition>
  name = <item-name>
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: #PCDATA | ttm:item*
</ttm:item>
```

`ttm:name` (§14.1.7):
```
<ttm:name
  condition = <condition>
  type = ("full" | "family" | "given" | "alias" | "other")
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  xml:space = ("default" | "preserve")
  Content: #PCDATA
</ttm:name>
```

`ttm:role` (attribute, §14.2.2, applies to `ttm:agent`-referenced content —
whitespace-separated role tokens):
```
ttm:role : role (<lwsp> role)*
role     : "action"|"caption"|"description"|"dialog"|"expletive"|"kinesic"|"lyrics"|"music"
         | "narration"|"quality"|"sound"|"source"|"suppressed"|"reproduction"|"thought"|"title"
         | "transcription" | extension-role
extension-role : "x-" token-char+     // token-char: XML NameChar
```

Metadata value expressions (§14.3):
```
<item-name>   : <named-item> | xsd:QName
<named-item>  : "altText" | "usesForced"
```
(`altText`/`usesForced` are the two IMSC-defined named metadata items — see
`imsc11-profiles.md`.)

### 3.10 Intermediate Synchronic Document (ISD) vocabulary (Appendix J,
informative — the output shape of the "flattening" transformation, not
authored input; listed here because a transmux-style processor that produces
ISDs for internal use would need this shape)

```
<isd:sequence
  extent = xsd:string
  size = xsd:nonNegativeInteger
  version = xsd:positiveInteger
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  {any attributes in the ISD Parameter Attribute Set}
  Content: ttm:metadata*, ttp:profile?, isd:isd*
</isd:sequence>

<isd:isd
  begin = <time-expression>
  end = <time-expression> | "indefinite"
  extent = xsd:string
  version = xsd:positiveInteger
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  {any attributes in the ISD Parameter Attribute Set}
  Content: ttm:metadata*, ttp:profile?, isd:css*, isd:region*
</isd:isd>

<isd:css
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  {any attributes in TT Style Namespaces}
  Content: ttm:metadata*
</isd:css>

<isd:region
  style = IDREF
  ttm:role = xsd:string
  xml:base = <uri>
  xml:id = ID
  xml:lang = xsd:string
  Content: ttm:metadata*, animate*, body
</isd:region>
```

## 4. Attribute vocabulary quick index (Table 5-5, §5.4.1)

Global/shared attributes by category (informative grouping, matching the
spec's own table so an implementer can sanity-check completeness):

| Module | Attributes |
|---|---|
| Animation Binding | `animate` |
| Conditionalization | `condition` |
| Core | `xml:base`, `xml:id`, `xml:lang`, `xml:space` |
| Data | `encoding`, `format`, `src`, `type` |
| Layout Binding | `region` |
| Linking | `xlink:arcrole`, `xlink:href`, `xlink:role`, `xlink:show`, `xlink:title` |
| Metadata | `ttm:agent`, `ttm:role` |
| Parameter | `ttp:cellResolution`, `ttp:clockMode`, `ttp:displayAspectRatio`, `ttp:dropMode`, `ttp:frameRate`, `ttp:frameRateMultiplier`, `ttp:markerMode`, `ttp:pixelAspectRatio`, `ttp:subFrameRate`, `ttp:tickRate`, `ttp:timeBase` |
| Profile | `ttp:contentProfiles`, `ttp:contentProfileCombination`, `ttp:inferProcessorProfileMethod`, `ttp:inferProcessorProfileSource`, `ttp:permitFeatureNarrowing`, `ttp:permitFeatureWidening`, `ttp:processorProfiles`, `ttp:processorProfileCombination`, `ttp:profile`, `ttp:validation`, `ttp:validationAction` |
| Style Binding | `style` |
| Styling | all 52 `tts:*` properties in the §3.5 table above |
| Timing | `begin`, `dur`, `end`, `timeContainer` |

Note: this table lists only attributes that are either global (namespace
qualified) or shared element-specific (unqualified but used across multiple
element types) — per-element-only attributes (e.g. `combine` on `ttp:profile`)
are documented only in their element's syntax box in §3.

## 5. Conformance model (§3, summary — see spec text for full normative wording)

- **Document conformance** (§3.1): a *timed text document instance* using only
  vocabulary/semantics permitted by this spec (or a profile of it).
- **Generic processor conformance** (§3.2.1): baseline requirements any
  conforming processor (of either kind below) must meet.
- **Transformation processor** (§3.2.2): processes a document without
  presenting it (e.g. a validator, or — relevant to this workspace — a
  transmux-style repackager).
- **Presentation processor** (§3.2.3): renders a document instance.

## 6. Not transcribed here (out of scope for a container/subtitle-carriage
crate, or covered elsewhere)

- Full CSS-style resolution algorithm (§10.4) and rendering/line-layout model
  (§11.3) — presentation-processor concerns, not parse/serialize concerns.
- Concrete encoding / reduced XML infoset (Appendix A, B) — general XML
  processing model, not TTML-specific.
- RNC/XSD schemas (Appendix C) — formal grammars redundant with the syntax
  boxes above.
- Root Container Region aspect-ratio derivation algorithm (Appendix H) — cited
  by reference above where relevant (`ttp:displayAspectRatio`,
  `ttp:pixelAspectRatio`) but not reproduced in full.
- Time Expression Semantics prose beyond §12.3.1/§12.4 (Appendix I) — the
  three time-base interpretations (clock/media/smpte) reference this appendix
  for full detail.
- Non-normative appendices (K–V): references, requirements, vocabulary
  derivation rationale, QA framework, security/privacy, HDR compositing,
  streaming fragmentation, styling examples, presentation-customisation
  options, TTML1→TTML2 diff, acknowledgements.
