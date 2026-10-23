# tshttp-runtime

Sans-IO TS-over-HTTP pull session client.

Bytes in, actions out. The host performs all I/O; this crate owns the session
state machine, demuxing via `transmux::StreamingTsDemux`, backoff and stall
detection.

`#![no_std]` + `alloc`; time is a caller-supplied monotonic millisecond count.
