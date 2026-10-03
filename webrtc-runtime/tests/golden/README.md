# WHIP/WHEP HTTP golden

`whip_whep_http.golden` is every HTTP request/response the WHIP/WHEP state
machines emit, generated on `main` at BASE `182d03f78bd508981c2f2dace5efeebcfe8823d6`
(before any W1-R-low-b change) with:

```text
GOLDEN_UPDATE=1 cargo test --locked -p webrtc-runtime --all-features --test golden_http
```

`tests/golden_http.rs` compares the live output to this file byte-for-byte.
