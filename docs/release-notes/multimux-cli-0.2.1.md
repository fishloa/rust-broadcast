# multimux-cli 0.2.1

Patch release — dependency bump only.

## Changed

- Bump the `multimux` dependency to 0.4 (which adds the RTMP push ingest input,
  `InputSpec::Rtmp`). No change to the CLI surface; an `rtmp` input is now
  configurable via the JSON route config.
