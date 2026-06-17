# ETSI TS 102 727 V1.1.1 (2010-01) — DVB-MHP 1.1.3: descriptor syntax (subset)

Transcribed (via BlazeDocs OCR, spot-checked against the vendored PDF) for the
descriptors dvb-si had deferred for lack of a syntax table (issue #227):

- **DVB-J application descriptor** (AIT application-descriptor namespace, tag `0x03`) — §10.9.1, PDF p.171.
- **DVB-J application location descriptor** (AIT application-descriptor namespace, tag `0x04`) — §10.9.2, PDF p.171.
- **xait_pid_descriptor** (extension descriptor `0x7F` / `descriptor_tag_extension` `0x0C`) — §10.17.3, PDF p.184.

The AIT application-descriptor namespace tags `0x00`–`0x05` (application, application_name,
transport_protocol, external_application_authorisation) are already implemented in
`dvb-si/src/descriptors/ait/` (issue #211); these add the DVB-J pair `0x03`/`0x04`.
(In the HbbTV-era TS 102 809, `0x03`/`0x04` are "reserved to DVB"; their DVB-J meaning is the MHP one below.)

---
