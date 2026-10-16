## Table 93 — Provider export descriptor syntax
_§10.15, PDF pp. 181-181 (printed p.181). Verified on the rendered page (PDF p.181, printed p.181)._

| No. of Bits | Identifier | Value |
|---|---|---|
| provider_export_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf | 0x12 |
| descriptor_length | 8 | uimsbf |  |
| For(i=0;i<N;i++) { |  |  |
| provider_name_length | 8 | uimsbf |  |
| For(j=0; i<N; i++) { |  |  |
| provider_name_byte | 8 | uimsbf |  |
| } |  |  |
| provider_class_name_length | 8 | uimsbf |  |
| For(j=0; i<N; i++) { |  |  |
| provider_class_name_byte | 8 | uimsbf |  |
| } |  |  |
| } |  |  |
| } |  |  |

Used in the application (inner) loop of the AIT for an application which exports (contains) an
XletBoundProvider; "meaningless if used in the common (outer) loop".

- provider_name_length: "the number of bytes in the provider name".
- provider_name_byte: "a string specifying the name of the provider".
- provider_class_name_char_length (sic; the table names the field provider_class_name_length):
  "the number of bytes in the application name" (sic).
- provider_class_name_byte: "a string specifying the fully qualified name of the class in the
  application that implements the named provider".

> **Spec note:** the table prints both inner loops as `For(j=0; i<N; i++)`: the loop variable
> `j` is initialised but the bound/increment use `i`, a typo. The implemented reading is the only
> one consistent with the field semantics: the `provider_name_byte` loop runs
> `provider_name_length` times and the `provider_class_name_byte` loop runs
> `provider_class_name_length` times; the outer `i<N` loop is bounded by `descriptor_length`.
> The prose also calls the class-name length field `provider_class_name_char_length` and says it
> counts "bytes in the application name"; the clause means the class name. No character set is
> specified for either string, so they are carried as opaque bytes.
