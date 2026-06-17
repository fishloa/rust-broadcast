## Table 45 — Reference type coding
_§9.4.3, PDF pp. 72-72_

<!-- Auto-transcription captured only value 0; rows 1–15 hand-completed verbatim from the PDF (2026-06-12). -->

| reference_type | Description |
|---|---|
| 0 | reserved for future use |
| 1 | The payload section is located on the same elementary stream and is identified by a table_id value carried in the first reference_byte and by the section_hash itself. The reference_length field shall be set to 1. |
| 2 | The payload section is located on a different elementary stream and is identified by component_tag and table_id values carried in the first and second reference_byte respectively, and by the section_hash itself. The referenced elementary stream can be found by looking up the component_tag value in a stream_identifier_descriptor within the PMT of the service carrying the authentication message section. The reference_length field shall be set to 2. |
| 3 to 15 | reserved for future use |

