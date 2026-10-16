## Table 64 — Deferred_association_tags_descriptor
_§11.3.3, PDF pp. 73-73. Verified on the rendered page (PDF p.73, printed p.73)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| deferred_association_tags_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| association_tags_loop_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| association_tag | 16 | uimsbf |
| } |  |  |
| transport_stream_id | 16 | uimsbf |
| program_number | 16 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

descriptor_tag: "It shall have the decimal value of 21" (0x15). association_tags_loop_length:
"the length in bytes of the loop of association tags that follows this field". association_tag:
"associated with either a stream that is not part of this data broadcast service or another DVB
service". transport_stream_id: the TS in which the associated service resides. program_number:
"shall be set to the service_id of the service". private_data_byte: "shall contain the
deferred_service_location structure which is defined in Table 65".

## Table 65 — Syntax for deferred_service_location structure
_§11.3.3, PDF p.73. Verified on the rendered page (PDF p.73, printed p.73)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| deferred_service_location() { |  |  |
| org_network_id | 16 | uimsbf |
| for (i=0; i<N, i++) { |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

org_network_id: "the network_id of the delivery system from which the service originates".
private_data_byte: "not specified by the present document".

> **Spec note:** Table 65 prints the loop as `for (i=0; i<N, i++)` (comma for semicolon); read as
> the usual `i<N; i++` loop over the remainder of the structure. Both loops are bounded by
> `descriptor_length`; the first (association_tag) loop by `association_tags_loop_length` (bytes, so
> an even count). ETSI TS 102 809
> B.3.3 (and TS 102 727 B.3.3) do not restate the layout; they add one semantic rule: a
> `transport_stream_id` of 0x0000 "shall be ignored in resolving the reference" (the receiver
> chooses the transport stream). The field is kept verbatim on the wire.
> A descriptor whose private bytes are empty (no `org_network_id`) is accepted and round-tripped;
> a lone single private byte cannot carry the 16-bit `org_network_id` and is rejected.
