# dvb-ci-runtime 0.14.1

Patch release. One fix to the #763 CAS-layer entitlement re-query.

## Fixed

**Re-query list-management now reflects the current active set (#765).** The
periodic `ca_pmt` re-query (`Driver::set_requery_interval`) previously resent
each service's `ca_pmt` with the `CaPmtListManagement` value frozen at
`add_service` time. After a `remove_service`, a sole surviving service could
re-query with a stale `Add` (no preceding `First`/`Only` in the list), which a
strictly-conformant CAM may reject.

The re-query now rebuilds every active service's `ca_pmt` fresh on each tick
(from the already-stored raw PMT), assigning `CaPmtListManagement` by position
in the current active set — `Only` for a single service, `First`/`More`/`Last`
across a multi-service set (EN 50221 §8.4.3.4, Table 25), with
`ca_pmt_cmd_id = query`. The internal per-service `requery_ca_pmt` byte-copy is
removed (re-query is rebuilt from `pmt_raw`).

No API changes. Additive/compatible: 0.14.0 → 0.14.1.
