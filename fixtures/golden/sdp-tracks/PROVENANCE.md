# sdp-tracks goldens

`broadcast-client.txt` and `multimux.txt` record what each crate's SDP media-section interpreter
returned over every `.sdp` fixture in the repo (`test_fixtures::sdp_track_fixtures`) plus the
hand-written cases in `in/` (own work, no third-party content; the sprop-parameter-sets are the
ones already used by `rtsp_pull_golden`). Regenerate with `REGENERATE_SDP_TRACKS_GOLDEN=1`
(refused when `CI` is set); a diff is the interpretation changing and needs a reason.
