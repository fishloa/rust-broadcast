# HLS client URL golden

`client_urls.golden` is the HLS client's actions and request URLs, generated on
`main` at BASE `182d03f78bd508981c2f2dace5efeebcfe8823d6` (before any
W1-R-low-b change) with:

```text
GOLDEN_UPDATE=1 cargo test --locked -p hls-runtime --all-features --test golden_client
```

`tests/golden_client.rs` compares the live output to this file byte-for-byte.
