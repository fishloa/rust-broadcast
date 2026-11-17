# sdp-tracks goldens

`broadcast-client.txt` and `multimux.txt` record what each crate's SDP media-section interpreter
returned over every `.sdp` fixture in the repo (`test_fixtures::sdp_track_fixtures`) plus the
hand-written cases in `in/` (own work, no third-party content; the sprop-parameter-sets are the
ones already used by `rtsp_pull_golden`). Regenerate with `REGENERATE_SDP_TRACKS_GOLDEN=1`
(refused when `CI` is set); a diff is the interpretation changing and needs a reason.

## Independent oracle

The number of media sections in each fixture is cross-checked against `sdp-types`' own parse of the
same bytes (`accepted_media_indices_match_shared_golden`), so a fixture the interpreter invents a
section for disagrees with the third-party parse. Which sections are playable is this crate's
reading of RFC 2326 §15.1 and has no third-party reader in the repo, so that part pins stability,
not correctness.

`REGENERATE_SDP_TRACKS_GOLDEN` refuses to write when `CI` is set, so a re-record cannot ride along
in a CI run.
