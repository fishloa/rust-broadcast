## Table 86 — DVB-HTML application descriptor syntax
_§10.10.1, PDF pp. 172-172_

| No. of Bits | Identifier | Value |
|---|---|---|
| dvb_html_application_descriptor(){ |  |  |
| descriptor_tag | 8 | uimsbf |  |
| descriptor_length | 8 | uimsbf |  |
| appid_set_length | 8 | uimsbf | N1 |
| for(i=0; i<N1; i++) { |  |  |  |
| application_id | 16 | bslbf |  |
| } |  |  |  |
| for(j=0; j<N; j++) { |  |  |  |
| parameter_bytes | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

descriptor_tag = 0x08. _Verified on the rendered page (PDF p.172, printed p.172)._ The printed table also puts `N1` in
the Value column of the `appid_set_length` row.

> **Spec note (deferred, not implemented; a determinate reading needs the owner's call).** Two statements on
> p.172 do not fix the unit of `appid_set_length`:
> 1. Table 86 bounds the loop as `for(i=0; i<N1; i++) { application_id (16, bslbf) }` with `N1` shown as the
>    value of `appid_set_length`, i.e. a **count of 16-bit application_ids**;
> 2. the prose says verbatim "appid_set_length: This 8 bit integer specifies the length of the list of
>    application_ids." with no unit (entries? bytes?), i.e. possibly a **byte length** (2 x N1 bytes).
>
> The two readings differ by a factor of two for every non-empty descriptor, and the trailing
> `for(j=0; j<N; j++) { parameter_bytes }` run (bounded by `descriptor_length`) depends on which one is used.
> The loop bound as printed and the prose each have a claim; neither the table nor any other vendored clause
> says which governs, and no real capture carries tag 0x08, so the descriptor stays unimplemented
> (`AnyAitDescriptor::Unknown`, bytes kept) until the owner decides the reading.
