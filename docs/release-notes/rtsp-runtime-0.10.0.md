# rtsp-runtime 0.10.0

_Released 2026-11-08._

Breaking-class release (0.9 -> 0.10): `Error` keeps the source errors it used to flatten into strings, and as a result no longer derives `Clone`, `PartialEq` or `Eq`.

Version: 0.9.0 -> 0.10.0.

## Breaking changes and migration

Variants that carried a `String` made with `to_string()` lost the original error, its `kind()` and its `source()` chain (#1726). They now carry the source, with a typed label next to it, and `Display` prints only the error's own context so a reporter that walks `source()` prints each message once:

```text
Error::MessageParse(String)  ->  Error::MessageParse { what: MessageKind, source: Box<dyn Error + Send + Sync> }
Error::MessageWrite(String)  ->  Error::MessageWrite(rtsp_types::WriteError)
Error::Tls(String)           ->  Error::Tls { server_name: String, source: Box<dyn Error + Send + Sync> }
Error::Io(String)            ->  Error::Io { context: IoContext, source: std::io::Error }
Error::Auth(..)              ->  same shape; Display no longer repeats the broadcast_auth::Error
```

`MessageKind` is `Request`, `Response` or `Framing` (the codec's or the framing check's error). `IoContext` is the closed set `Connect`, `TlsHandshake`, `Read` and `Socket` (the last for a plain `From<std::io::Error>` conversion). Both enums, and `Error`, are `#[non_exhaustive]`, have `name()` and `Display`, and are re-exported at the crate root.

Messages change. `Io` was `socket IO error: <detail>` and is now `socket IO error during <context>`; the underlying `std::io::Error` is `source()`. `MessageParse` is `failed to parse RTSP <request|response|message framing>`, `MessageWrite` is `failed to serialize RTSP message`, `Tls` is `TLS error: invalid server name "<name>"` and `Auth` is `authentication failure`. Code that logged `{err}` and relied on the detail being in that text must print the chain:

```rust
// before: the detail was inside the message
log::warn!("rtsp: {err}");

// after: walk source() (or use an error reporter) to see the detail
let mut line = err.to_string();
let mut cause = std::error::Error::source(&err);
while let Some(c) = cause {
    line.push_str(": ");
    line.push_str(&c.to_string());
    cause = c.source();
}
log::warn!("rtsp: {line}");
```

Matching on the new shapes:

```rust
// before
match err {
    Error::Io(msg) => log_io(&msg),
    Error::MessageParse(msg) => log_parse(&msg),
    Error::Tls(msg) => log_tls(&msg),
    _ => {}
}

// after
match err {
    Error::Io { context: IoContext::Connect, source } => retry_connect(source.kind()),
    Error::Io { context, source } => log_io(context.name(), source.kind()),
    Error::MessageParse { what, source } => log_parse(what.name(), &source.to_string()),
    Error::Tls { server_name, source } => log_tls(&server_name, &source.to_string()),
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
