# transmux 0.32.1

_Released 2026-11-17._

### Fixed
- **Overlapping samples can no longer each copy the same bytes (quadratic memory).** `progressive_demux::sample_layout` bounded only `stsz.sample_count` against the file length, so `stco` entries all pointing at one region with a uniform `stsz` made `sample_count x sample_size` samples — `file_len^2` bytes from a `file_len` input — and every one of them copied those same bytes again. The expander now accumulates the resolved sample bytes and refuses a total past `frag_offsets::MAX_SAMPLE_BYTES_PER_INPUT_LENGTH` times the input length (samples that do not overlap can never total more) with the typed `Error::SampleDataBudgetExceeded { limit }`; the protected-progressive path of `CencDecryptor` expands through the same function and inherits the bound (audit 20 finding 6, #2817).
- **The fMP4 sample-byte bound is per run, not per `moof`.** `Fmp4Demux` and `CencDecryptor` capped each fragment's sample total against the file length, so one large `mdat` followed by N tiny `moof`s that each name it in `tfhd.base_data_offset` passed every check and copied the whole payload N times (tens of GB from a few MB). Both now thread one never-refilled counter through the run (`frag_offsets::sample_ranges_in_run`) and refuse a fragment past `MAX_SAMPLE_BYTES_PER_INPUT_LENGTH` times the input length with `Error::SampleDataBudgetExceeded`; `Fmp4Demux` records such a fragment in `Media::degraded` and keeps the samples before it, while `CencDecryptor::demux` returns the error (audit 20 finding 13, #2824).
- **The `mdat` containment check is a binary search.** Every resolved sample asked whether it lies inside some `mdat` payload by scanning the file's whole payload list, so a file of 8-byte empty `mdat`s and a `trun` of zero-size samples cost O(samples x mdats) — 2e11 comparisons for a 1.6 MB input. `mdat_ranges` is file-ordered and disjoint, so only the first payload whose end reaches the sample can contain it: `partition_point` makes the check O(log mdats) (audit 20 finding 39, #2850).

---

Published from tag `transmux-v0.32.1`.
