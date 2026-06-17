## compressed_module_descriptor (tag 0x09)
_TR 101 202 §4.6.6.10, PDF p. 20 — appears in the ModuleInfo `userInfo` loop_

Standard DVB descriptor framing (`descriptor_tag` 8, `descriptor_length` 8) then
the body. The DVB guideline is that the module bytes are zlib-compressed; the
zlib payload structure (RFC 1951 DEFLATE wrapped per RFC 1950) is:

| Field | bytes | Comment |
|---|---|---|
| `compression_method` | 1 | zlib CMF (RFC 1950) |
| `flags_check` | 1 | zlib FLG |
| `compressed_data` | n | DEFLATE stream (RFC 1951) |
| `check_value` | 4 | Adler-32 |

Decompression is gated behind the optional **`flate2`** feature (off by default);
without it the compressed module bytes are exposed raw.

