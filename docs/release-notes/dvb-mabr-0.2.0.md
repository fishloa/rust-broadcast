# dvb-mabr 0.2.0

_Released 2026-10-05._

Breaking (0.x minor). The XML layer is rewritten on `quick-xml` (the hand-rolled `roxmltree`-based parsing is gone), which has three consequences: the whole XML API now requires the `std` feature, a handful of documents that parsed before are now rejected, and `to_xml` output changes in one narrow case. For the committed Annex C fixtures, parse results and serialized output are identical to 0.1.0. Who must act: anyone building with `--no-default-features` (the XML types disappear), anyone parsing untrusted or foreign-namespace documents, and anyone relying on a `schemaVersion="2"` document being rewritten to the 2024 namespace. The previous published version is 0.1.0.

## Breaking changes

### XML API requires `std`

`MulticastServerConfiguration`, `MulticastGatewayConfiguration` and every element type are now behind the `std` feature (on by default). A `--no-default-features` build exposes only the `error` module (`Error`, `Result`). The `no-std` crates.io category was dropped.

```toml
# 0.1.0: worked without std (no_std + alloc)
dvb-mabr = { version = "0.1", default-features = false }
# 0.2.0: default features (or at least `std`) are required to use the XML API
dvb-mabr = "0.2"
```

### New public field and type

`MulticastServerConfiguration` and `MulticastGatewayConfiguration` gain `namespace: BaselineNamespace` (both structs are `#[non_exhaustive]`, so downstream code could not construct them by literal anyway). `BaselineNamespace` is new, `#[non_exhaustive]`, with variants `V2019` and `V2024` (default) and `uri()` / `name()` / `Display`. It records which baseline namespace the root declared, and is re-exported at the crate root.

## Behaviour changes

Documents that parsed in 0.1.0 and are now rejected (all follow XML 1.0 or the schema):

- **Root namespace is checked (#1121).** A `MulticastServerConfiguration` or `MulticastGatewayConfiguration` root in any namespace other than the 2019 or 2024 baseline fails with `Error::UnexpectedRoot`. Before, only the local name was checked.
- **`ReportingLocator::proportion` is range-checked (#1121).** A non-finite value (`NaN`, `inf`, `infinity` all parse as `f64` but are not valid `xs:decimal`/`xs:double` lexical forms) or a value outside `(0.0, 1.0]` is rejected. `NaN != NaN` also broke the documented parse, `to_xml`, parse round trip.
- **Control characters.** A numeric reference to a control character (`&#x1;`) or a literal one, in text or in an attribute, is rejected with `Error::XmlParse` (outside the XML 1.0 `Char` production).
- **Entity reference or CDATA before the root element** is rejected (`&amp;<MulticastGatewayConfiguration ...>` is `Error::XmlParse`).
- **More than 128 namespace declarations in scope at once** is rejected with `Error::XmlParse` (quick-xml's namespace-binding limit).
- A `DOCTYPE`, an undefined entity, an unbound namespace prefix, a second root element, or content after the root are rejected with `Error::XmlParse`. Malformed input stays a structured `Error::XmlParse`; do not match on its message text, which is produced by the new parser.

Output differences:

- **`to_xml` re-emits the namespace the document declared (#1121).** 0.1.0 always wrote the 2024 namespace, producing a contradictory `xmlns="...:2024"` with `schemaVersion="1"` when re-serializing a v1 document, and rewriting a `schemaVersion="2"` document that declared the 2019 namespace. The round trip is now byte-stable for both.
- **Attribute whitespace is written as character references.** An attribute value containing `\t`, `\n` or `\r` is written as `&#9;`, `&#10;`, `&#13;` (`serviceIdentifier="a\nb"` renders `serviceIdentifier="a&#10;b"`) so it survives a re-parse. Every attribute value and text node is now escaped by quick-xml.

Parsing is a single pass over `quick_xml::NsReader` events straight into the typed structs, with no document tree, so hostile nesting cannot overflow the stack.

## Fixes

- **Extension elements in a foreign namespace (#1121).** `child`/`children` now match `(namespace, local name)` instead of the local name alone, so a private extension element (Annex A.1) in a foreign namespace that reuses a baseline local name is skipped, as the crate documents, instead of being parsed as the baseline element.
- **Element text truncated at a comment or CDATA boundary (#1121).** Leaf URI/string elements now concatenate every text-node child; the `roxmltree` `Node::text()` call used before returned only the first.

## Dependencies

Verified from the `Cargo.toml` diff against `dvb-mabr-v0.1.0`:

```toml
broadcast-common = { version = "9.3" -> "9.4", default-features = false }
roxmltree        = { version = "0.20", default-features = false }                # removed
quick-xml        = { version = "0.42", default-features = false, optional = true }  # added
std = ["broadcast-common/std", "thiserror/std", "roxmltree/std"]
  -> ["broadcast-common/std", "thiserror/std", "dep:quick-xml"]
categories: "no-std" removed
```

quick-xml is the single XML dependency across the workspace crates that read or write XML (transmux, dvb-mabr, ttml-subtitle, multimux); its being `std`-only is the reason for the feature gate.

---

Published from tag `dvb-mabr-v0.2.0`.
