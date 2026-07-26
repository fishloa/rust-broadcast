# ll-hls-runtime 0.1.1

Patch release — dependency bump only.

## Changed

- Bump the `transmux` dependency to 0.19 (adds `StreamingFlvDemux`; no API or
  behavior change to `ll-hls-runtime`). Keeps a single `transmux` version across
  the `multimux` dependency tree for the RTMP-ingest wave.
