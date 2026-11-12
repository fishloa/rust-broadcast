# PROVENANCE

## SRT push URL goldens

`push-url-table.txt` records how `multimux`'s SRT push parses every `srt://`
URL shape it accepts or rejects: host only, `host:port`, bracketed IPv6 with
and without a port, bare IPv6 (a deliberate error), userinfo (a deliberate
error), `streamid` plain and percent-encoded, `latency` 0/200/8000/8001 and
non-numeric, `mode=caller`, `mode=listener` (a deliberate error),
`passphrase` (a deliberate error), an unknown query key, a trailing slash and
an empty query pair. Each line is `<url>\t<outcome>`.

Taken from `main` at commit 2ee7f4c52a06662e8b50480dd1ecf66ba660848d, before
the parser moved from `multimux` into `srt-runtime` (issue #2369). Regenerated
only by the test that owns it:

    REGENERATE_PUSH_URL_TABLE=1 cargo test -p multimux --all-features --locked --test it push_wire_goldens

It is never re-blessed to make a change pass.
