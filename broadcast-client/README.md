# broadcast-client

Generic sans-IO client session for live media: auth, redirects, and a TS-over-HTTP
source on `tshttp-runtime`.

## Quick start

```rust
use broadcast_client::{Config, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
while let Some(action) = session.poll(0) {
    // host performs I/O based on `action`
    let _ = action;
}
```
