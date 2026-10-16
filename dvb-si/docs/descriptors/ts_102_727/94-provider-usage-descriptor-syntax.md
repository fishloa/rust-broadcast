## Table 94 — Provider usage descriptor syntax
_§10.15, PDF pp. 181-181 (printed p.181). Verified on the rendered page (PDF p.181, printed p.181)._

| No. of Bits | Identifier | Value |
|---|---|---|
| provider_usage_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf | 0x13 |
| descriptor_length |  |  |  |
| For(i=0;i<N;i++) { |  |  |
| provider_name_length |  |  |  |
| For(j=0; i<N; i++) { |  |  |
| provider_name_byte |  |  |  |
| } |  |  |
| } |  |  |
| } |  |  |

Used in either the application (inner) or common (outer) loop of the AIT for an application
which uses an XletBoundProvider and does not include the classes of the provider as part of itself.

- provider_name_length: "the number of bytes in the provider name" (stated as an 8 bit unsigned integer).
- provider_name_byte: "a string specifying the name of the provider".

> **Spec note:** as printed, the table gives the width/mnemonic only for `descriptor_tag`
> (8, uimsbf); the other rows are blank (the prose states `provider_name_length` is an 8 bit
> unsigned integer; `descriptor_length` is 8 bits as in every descriptor and in Table 93). The inner
> loop is printed `For(j=0; i<N; i++)`, the same `j`/`i` typo as Table 93; it is implemented as
> `provider_name_length` repetitions. Outer loop bounded by `descriptor_length`.
