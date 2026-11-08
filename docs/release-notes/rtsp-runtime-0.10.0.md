# rtsp-runtime 0.10.0

_Released 2026-11-08._

Breaking-class release (0.9 -> 0.10): `Error` keeps the source errors it used to flatten into strings, and as a result no longer derives `Clone`, `PartialEq` or `Eq`.

Version: 0.9.0 -> 0.10.0.

## Breaking changes and migration

Four variants carried a `String` made with `to_string()`, which lost the original error, its `kind()` and its `source()` chain (#1726). They now carry the source:

```text
Error::MessageParse(String)  ->  Error::MessageParse(Box<dyn std::error::Error + Send + Sync>)
Error::MessageWrite(String)  ->  Error::MessageWrite(rtsp_types::WriteError)
Error::Tls(String)           ->  Error::Tls(Box<dyn std::error::Error + Send + Sync>)
Error::Io(String)            ->  Error::Io { context: &'static str, source: std::io::Error }
```

`Io::context` names the operation that failed (`"connect"`, `"TLS handshake"`, `"read"`, `"write"`, or `"socket"` for a plain `From<std::io::Error>` conversion). `Display` of `Io` changes from `socket IO error: ...` to `socket IO error during <context>: ...`; the other three keep their message text.

Matching on the new shapes:

```rust
// before
match err {
    Error::Io(msg) => log_io(&msg),
    Error::MessageParse(msg) => log_parse(&msg),
    _ => {}
}

// after
match err {
    Error::Io { context, source } => {
        log_io(context, source.kind());
    }
    Error::MessageParse(source) => log_parse(&source.to_string()),
    _ => {}
}
```

Because the variants now hold non-`Clone`, non-`PartialEq` types, `Error` no longer derives `Clone`, `PartialEq` or `Eq`. Code that cloned an `Error`, stored it in a `PartialEq` struct, or compared two errors with `==` must change:

```rust
// before
assert_eq!(result, Err(Error::MissingCSeq));

// after
assert!(matches!(result, Err(Error::MissingCSeq)));
```

To keep an error beyond its owner, store `err.to_string()`, or the `source()` you need, instead of cloning it.

## Fixed

- The source error of a failed parse, write, socket operation or TLS setup is reachable through `std::error::Error::source()` (#1726).

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `rtsp-runtime-v0.10.0`.
