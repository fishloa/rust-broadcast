# isobmff-private-format

A worked example of extending [`isobmff`](../..//isobmff) from **outside** the
crate: no `isobmff` source file changes, no fork, no upstream patch.

The crate defines two private wire types:

| Type | Four-CC | Where it lives | Body |
|------|---------|----------------|------|
| `AcmeCalibrationBox` | `acal` | `moov/udta` | FullBox: gain and shift (16.16 fixed point) and a counted label |
| `AcmeTelemetryEntry` | `actm` | `stsd` of a `meta` track | reserved + `data_reference_index`, then one 32-bit serial |

## How the plumbing works

1. Implement `Parse` and `Serialize` exactly as any box in `isobmff` does. The
   parser receives the **complete box bytes**, header included, and the
   serializer writes them back.
2. Implement `BoxDef`:

   ```rust,ignore
   impl BoxDef for AcmeCalibrationBox {
       const FOURCC: [u8; 4] = *b"acal";
       const PARENT: Option<[u8; 4]> = Some(*b"udta"); // scoped to udta
   }

   impl BoxDef for AcmeTelemetryEntry {
       const FOURCC: [u8; 4] = *b"actm";
       const PARENT: Option<[u8; 4]> = Some(*b"stsd"); // a sample entry
   }
   ```

   `PARENT = None` registers the type in every parent; `UUID` instead of
   `FOURCC` addresses a `uuid` box by its extended type.
3. Build a registry and hand it to the reader:

   ```rust,ignore
   let registry = acme_registry();
   let reader = Reader::new(ReaderConfig::default().with_registry(registry));
   ```

4. A matching `stsd` entry comes back in `SampleEntryInfo::registered`; call
   `RegisteredEntry::downcast_ref::<AcmeTelemetryEntry>()` for the value. A
   matching `udta` child is found with
   `BoxRegistry::parse_children(*b"udta", payload)`.

## Writing it back

The writer needs no registry. `SampleEntry::from_registered` turns a registered
entry into the raw entry bytes the writer carries, and an opaque child keeps
whatever is in it:

```rust,ignore
let entry = SampleEntry::from_registered(parse.as_registered())?;
let setup = TrackSetup::new(track_id, *b"meta", b"acme telemetry".to_vec(), timescale, entry);
```

## What the test proves

`tests/round_trip.rs` builds a progressive file and a fragmented file over the
fixture's video track plus one private `actm` track, with an `acal` box inside
`moov/udta`, then:

- reads both back **with** the registry: the entry and the atom are typed and
  equal to what was written;
- reads both back **without** the registry: the entry has no typed form and no
  registered parse, the atom is opaque, and rewriting the file from the plan is
  **byte-identical** either way.

Run it with:

```bash
cargo test -p isobmff-private-format
```

## Registering over a type `isobmff` already owns

`BoxRegistry::register` refuses a four-CC this crate types itself (`stsd`,
`moov`, `avc1`, `mp4a`, …) and any key already registered. Use
`register_override` when replacing one deliberately, for example to read a
vendor's non-standard variant of a standard box in a known-odd capture.
