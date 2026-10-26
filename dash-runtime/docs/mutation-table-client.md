# Mutation table: `DashClient` (design step W-d1)

Each mutant is a one-line change to `src/client/` applied on its own, followed by
`cargo test -p dash-runtime --no-fail-fast` over the four client test files
(`client_static`, `client_periods`, `client_faults`, `client_misc`); every run
recompiled the crate (`Compiling dash-runtime` appeared) and the edit was then
reverted. A mutant is **killed** when at least one test fails. The last column
lists the first three failing tests.

Result: 48 of 48 mutants killed.

The first run left two survivors (C02: any segment, not only a short last one, may be a
tail; C23: the origin pin ignores the scheme) and one mutant that did not compile (C48,
rewritten). The tests that kill them (`a_short_segment_in_the_middle_is_not_a_tail`, the
`https://dash.test:80/` row of `cross_origin_urls_are_refused_when_pinned_and_never_requested`)
were added in the same change; C02, C23 and C48 were then re-run and are killed. `mutants_client.py`
in this directory holds the exact edits and re-runs them (`--dry` checks that every pattern
still matches exactly once). It is not part of the build.

The formulas of the core (`address`, `timing`, `selection`) have their own table in
`mutation-table.md`.

| Id | Mutation | File | Result | Killed by |
|---|---|---|---|---|
| C01 | tail threshold `<` -> `>=`: a short last segment is not a tail | `client/engine.rs` | killed | `a_gone_tail_ends_the_period_like_a_not_found_tail`, `a_missing_full_length_last_segment_is_not_a_tail`, `a_tail_that_fails_for_another_reason_is_retried_like_any_segment` |
| C02 | any segment, not only the last, may be a tail | `client/engine.rs` | killed | `a_short_segment_in_the_middle_is_not_a_tail` |
| C03 | the missing tail is never recognised | `client/engine.rs` | killed | `a_gone_tail_ends_the_period_like_a_not_found_tail`, `a_missing_full_length_last_segment_is_not_a_tail`, `a_tail_that_fails_for_another_reason_is_retried_like_any_segment` |
| C04 | 404/410 classified Server, not Missing | `client/engine.rs` | killed | `counted_failure_kinds_follow_the_status` |
| C05 | 425 classified Server, not TooEarly | `client/engine.rs` | killed | `counted_failure_kinds_follow_the_status` |
| C06 | give-up `>=` -> `>` (cap off by one) | `client/engine.rs` | killed | `a_cap_of_zero_or_one_makes_the_first_failure_final`, `a_missing_full_length_last_segment_is_not_a_tail`, `a_missing_middle_segment_is_not_a_tail` |
| C07 | failure counter never grows | `client/engine.rs` | killed | `a_body_over_the_size_cap_is_a_counted_failure`, `a_missing_full_length_last_segment_is_not_a_tail`, `a_missing_middle_segment_is_not_a_tail` |
| C08 | init/index use the segment cap | `client/engine.rs` | killed | `after_a_fatal_everything_is_ignored`, `init_and_segment_failure_caps_are_independent` |
| C09 | window `<` -> `<=` (one request too many) | `client/engine.rs` | killed | `a_representation_switch_continues_on_the_new_representation_and_reannounces`, `a_switch_that_is_not_segment_aligned_is_marked_with_a_discontinuity`, `completion_order_and_window_do_not_change_the_samples` |
| C10 | window of 0 not treated as 1 | `client/config.rs` | killed | `window_zero_behaves_as_one` |
| C11 | alternate rotation after 3 failures, not 2 | `client/engine.rs` | killed | `a_failing_alternate_is_rotated_away_after_two_failures`, `alternates_rotate_every_two_failures_in_a_fixed_order` |
| C12 | alternates never rotated | `client/engine.rs` | killed | `a_failing_alternate_is_rotated_away_after_two_failures`, `alternates_rotate_every_two_failures_in_a_fixed_order`, `alternates_that_all_fail_end_at_the_cap_having_tried_both` |
| C13 | consecutive-failure run not reset after a rotation | `client/engine.rs` | killed | `alternates_rotate_every_two_failures_in_a_fixed_order` |
| C14 | continuity signal ignored | `client/engine.rs` | killed | `continuity_naming_another_period_is_not_continuity`, `without_the_continuity_signal_the_same_boundary_is_a_discontinuity` |
| C15 | continuity ignores a changed init | `client/engine.rs` | killed | `continuity_with_a_different_init_url_is_not_continuous` |
| C16 | continuity ignores the AdaptationSet id | `client/engine.rs` | killed | `continuity_needs_the_same_adaptation_set_id` |
| C17 | no Discontinuity at a Period boundary | `client/engine.rs` | killed | `continuity_naming_another_period_is_not_continuity`, `continuity_needs_the_same_adaptation_set_id`, `continuity_with_a_different_init_url_is_not_continuous` |
| C18 | Discontinuity before the very first Period | `client/engine.rs` | killed | `a_representation_switch_continues_on_the_new_representation_and_reannounces`, `a_skipped_segment_is_one_discontinuity_and_the_rest_plays`, `a_switch_that_is_not_segment_aligned_is_marked_with_a_discontinuity` |
| C19 | Init re-announced across a continuous boundary | `client/engine.rs` | killed | `continuous_periods_join_without_a_discontinuity_or_a_second_init` |
| C20 | init identity keeps the URL query | `client/engine.rs` | killed | `init_identity_ignores_the_query` |
| C21 | origin pin never refuses | `client/engine.rs` | killed | `an_absolute_segment_template_cannot_escape_the_pin`, `cross_origin_urls_are_refused_when_pinned_and_never_requested` |
| C22 | origin comparison ignores the port | `client/engine.rs` | killed | `cross_origin_urls_are_refused_when_pinned_and_never_requested` |
| C23 | origin comparison ignores the scheme | `client/engine.rs` | killed | `cross_origin_urls_are_refused_when_pinned_and_never_requested` |
| C24 | MPD URL query appended to init and index too | `client/engine.rs` | killed | `the_mpd_url_query_is_appended_to_media_segments_only_when_the_mpd_asks` |
| C25 | a switch is always treated as aligned | `client/engine.rs` | killed | `a_switch_that_is_not_segment_aligned_is_marked_with_a_discontinuity` |
| C26 | switch continues one segment late | `client/engine.rs` | killed | `a_representation_switch_continues_on_the_new_representation_and_reannounces` |
| C27 | window slot never released on delivery | `client/engine.rs` | killed | `a_cross_origin_base_url_is_followed_by_default`, `a_failing_alternate_is_rotated_away_after_two_failures`, `a_gone_tail_ends_the_period_like_a_not_found_tail` |
| C28 | skipped segment leaves no Discontinuity | `client/engine.rs` | killed | `a_skipped_segment_is_one_discontinuity_and_the_rest_plays` |
| C29 | retry queued behind later requests | `client/engine.rs` | killed | `a_retried_resource_is_requested_before_anything_queued_behind_it` |
| C30 | newest segment requested first across lanes | `client/engine.rs` | killed | `with_a_window_of_one_requests_follow_the_media_timeline` |
| C31 | Period starts before its inits arrived | `client/engine.rs` | killed | `a_cross_origin_base_url_is_followed_by_default`, `a_failing_alternate_is_rotated_away_after_two_failures`, `a_gone_tail_ends_the_period_like_a_not_found_tail` |
| C32 | second lane reuses the first lane's track ids | `client/engine.rs` | killed | `a_cross_origin_base_url_is_followed_by_default`, `a_failing_alternate_is_rotated_away_after_two_failures`, `a_gone_tail_ends_the_period_like_a_not_found_tail` |
| C33 | segments held for an init are never demuxed | `client/engine.rs` | killed | `a_server_that_ignores_the_range_header_is_caught_on_the_init`, `a_transient_init_fault_recovers_with_every_sample`, `completion_order_and_window_do_not_change_the_samples` |
| C34 | a second delivery is not detected | `client/engine.rs` | killed | `protocol_misuse_is_a_typed_error_and_changes_nothing` |
| C35 | an unrequested delivery is accepted | `client/engine.rs` | killed | `protocol_misuse_is_a_typed_error_and_changes_nothing` |
| C36 | PeriodChanged carries start 0 | `client/engine.rs` | killed | `restart_boundary_is_one_discontinuity_before_the_new_periods_samples` |
| C37 | progressive MP4 segment not fatal | `client/engine.rs` | killed | `a_progressive_mp4_served_as_a_segment_is_fatal_at_once` |
| C38 | init without a complete moov accepted | `client/engine.rs` | killed | `rejection_reasons_are_typed` |
| C39 | structural media check bypassed | `client/engine.rs` | killed | `a_permanent_segment_fault_ends_after_the_cap_with_the_typed_fatal`, `a_transient_segment_fault_recovers_with_every_sample`, `counted_failure_kinds_follow_the_status` |
| C40 | size cap `>` -> `>=` | `client/engine.rs` | killed | `the_size_cap_accepts_a_body_of_exactly_the_cap` |
| C41 | ranged response length not checked | `client/engine.rs` | killed | `a_server_that_ignores_the_range_header_is_caught_on_the_init`, `a_short_or_overlong_sidx_response_is_a_length_rejection`, `a_short_subsegment_is_a_length_rejection_not_a_demux_of_half_a_fragment` |
| C42 | dynamic MPD accepted | `client/engine.rs` | killed | `a_dynamic_mpd_is_refused_cleanly_and_nothing_else_is_requested` |
| C43 | track set re-announced every Period | `client/engine.rs` | killed | `continuous_periods_join_without_a_discontinuity_or_a_second_init` |
| C44 | no encryption notice | `client/engine.rs` | killed | `a_protected_video_set_is_dropped_with_a_notice_and_the_audio_plays_alone_as_track_one` |
| C45 | failed resource cannot be retried | `client/engine.rs` | killed | `a_body_over_the_size_cap_is_a_counted_failure`, `a_failing_alternate_is_rotated_away_after_two_failures`, `a_missing_full_length_last_segment_is_not_a_tail` |
| C46 | byte ranges dropped from init/index requests | `client/engine.rs` | killed | `a_server_that_ignores_the_range_header_is_caught_on_the_init`, `a_short_or_overlong_sidx_response_is_a_length_rejection`, `a_short_subsegment_is_a_length_rejection_not_a_demux_of_half_a_fragment` |
| C47 | lane exhausted one segment late | `client/engine.rs` | killed | `a_cross_origin_base_url_is_followed_by_default`, `a_failing_alternate_is_rotated_away_after_two_failures`, `a_gone_tail_ends_the_period_like_a_not_found_tail` |
| C48 | dropped sets are not reported | `client/engine.rs` | killed | `a_protected_video_set_is_dropped_with_a_notice_and_the_audio_plays_alone_as_track_one`, `a_subtitle_set_is_reported_and_ignored`, `every_set_kind_the_client_cannot_play_is_reported_not_misplayed` |
