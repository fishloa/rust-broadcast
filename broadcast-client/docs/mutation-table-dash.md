# Mutation table: the DASH source (design step W-e)

Each mutant is a small change to production source (`source/dash.rs`, `session.rs`, and for `E` rows `dash-runtime/src/client/engine.rs`), applied on its own
and followed by `cargo test -p broadcast-client --all-features --locked --no-fail-fast` over the DASH test binaries (`dash_static`, `dash_live`, `dash_select`,
`dash_errors`, `dash_http`, `dash_muxed`, `dash_source_kind`, `dash_matrix`, plus `dash_clock_surface` and `label_coverage`; `E` rows first run
`dash-runtime`'s `client_select_audio` and `client_static`), then reverted. `mutants_dash.py` in this directory holds the exact edits (`--dry` checks that every
pattern matches exactly once). Every run recompiled the edited crate; a run that exceeds ten minutes is killed and counts as a kill (a mutant that makes the
simulation spin). A mutant is **killed** when at least one test fails.

Result: 78 of 79 mutants killed, 1 survives and is equivalent (below). The first run of this table left eleven survivors
(D12, D17, D19, D20, D22, D33, D35, D38, D54, S03, S05); each pointed at a test that could not fail and got a test that does (a `Retry-After` carried into a
later failure, the UTC and segment timeouts told apart by their instants, the body cap proved by an endless body being cut at the cap, features and
`LiveEdgeLost` once per session and per run, stopped audio filtered even when segments are already in flight, the MPD retry honouring `Retry-After`, the
playout gate across a variant switch, a clock given to a closed session). The table below is a complete re-run on the final commit.

| Id | Mutation | File | Result | Killed by (first four) |
|---|---|---|---|---|
| D01 | If-None-Match header name misspelt | `source/dash.rs` | killed | `the_mpd_refresh_is_conditional_and_a_304_is_accepted` |
| D02 | Last-Modified validator never sent | `source/dash.rs` | killed | `a_last_modified_validator_comes_back_as_if_modified_since_and_a_304_is_accepted` |
| D03 | a 304 answer is no longer taken as 'unchanged' | `source/dash.rs` | killed | `a_last_modified_validator_comes_back_as_if_modified_since_and_a_304_is_accepted`, `the_mpd_refresh_is_conditional_and_a_304_is_accepted` |
| D04 | UTCTiming HEAD sent as GET | `source/dash.rs` | killed | `utc_head_is_sent_with_the_head_method_and_a_refused_head_is_retried_as_get` |
| D05 | HEAD Content-Length counted as a truncated body | `source/dash.rs` | killed | `a_head_answer_declares_the_length_of_the_body_it_does_not_send` |
| D06 | range check skipped (any 2xx accepted for a ranged request) | `source/dash.rs` | killed | `a_truncated_mpd_transfer_is_a_transport_failure_not_a_parse_error`, `a_whole_file_answer_to_a_ranged_request_is_sliced_and_a_wrong_range_is_a_failure` |
| D07 | redirect count never grows | `source/dash.rs` | killed | `a_redirect_loop_is_bounded_per_attempt_and_ends_as_a_failed_segment` |
| D08 | 403 not reported as Forbidden | `source/dash.rs` | killed | `a_403_is_reported_and_the_request_is_retried` |
| D09 | 410 no longer 'not found' for the first MPD | `source/dash.rs` | killed | `only_the_first_mpd_not_found_or_gone_is_reported_as_not_found` |
| D10 | NotFound also after the MPD was accepted | `source/dash.rs` | killed | `only_the_first_mpd_not_found_or_gone_is_reported_as_not_found` |
| D11 | Retry-After ignored | `source/dash.rs` | killed | `an_init_that_keeps_failing_is_fatal_init_fetch_failed_and_retry_after_is_honoured`, `an_mpd_retry_waits_for_retry_after`, `retry_after_sets_the_wait_for_a_429_and_cookies_from_the_mpd_are_kept` |
| D12 | Retry-After kept for the next failure | `source/dash.rs` | killed | `retry_after_sets_the_wait_for_a_429_and_cookies_from_the_mpd_are_kept` |
| D13 | data before the response head enters the body | `source/dash.rs` | killed | `a_whole_file_answer_too_short_for_the_range_is_a_failure_and_data_before_the_head_is_ignored` |
| D14 | body cap not enforced | `source/dash.rs` | killed | `an_endless_body_is_cut_at_the_cap_and_the_connection_closed_at_once`, `an_oversize_mpd_is_reported_once_and_retried_forever_with_backoff`, `an_oversize_segment_body_is_reported_once_and_counted_as_a_failure` |
| D15 | oversize body reported every time | `source/dash.rs` | killed | `an_oversize_mpd_is_reported_once_and_retried_forever_with_backoff`, `an_oversize_segment_body_is_reported_once_and_counted_as_a_failure` |
| D16 | truncated transfer delivered as a complete body | `source/dash.rs` | killed | `a_truncated_mpd_transfer_is_a_transport_failure_not_a_parse_error` |
| D17 | range slice failure delivered | `source/dash.rs` | survived |  |
| D18 | timed-out request keeps its connection | `source/dash.rs` | killed | `a_hung_segment_is_closed_at_the_segment_timeout_and_requested_again`, `a_utc_request_is_abandoned_at_the_utc_timeout_and_an_init_at_the_request_timeout` |
| D19 | UTCTiming bounded by the manifest timeout | `source/dash.rs` | killed | `a_utc_request_is_abandoned_at_the_utc_timeout_and_an_init_at_the_request_timeout` |
| D20 | segment bounded by the manifest timeout | `source/dash.rs` | killed | `a_hung_segment_is_closed_at_the_segment_timeout_and_requested_again` |
| D21 | init and index bounded by the segment timeout | `source/dash.rs` | killed | `a_utc_request_is_abandoned_at_the_utc_timeout_and_an_init_at_the_request_timeout` |
| D22 | segment body cap taken from the manifest cap | `source/dash.rs` | killed | `an_endless_body_is_cut_at_the_cap_and_the_connection_closed_at_once` |
| D23 | MPD body cap taken from the segment cap | `source/dash.rs` | killed | `an_endless_body_is_cut_at_the_cap_and_the_connection_closed_at_once`, `an_oversize_mpd_is_reported_once_and_retried_forever_with_backoff` |
| D24 | unsupported MPD kind retried instead of fatal | `source/dash.rs` | killed | `a_list_presentation_is_fatal_unsupported_addressing` |
| D25 | MPD parse failures never fatal | `source/dash.rs` | killed | `an_mpd_that_is_not_xml_is_a_parse_error_reported_once_and_fatal_at_the_key_failure_limit`, `each_mpd_parse_failure_names_its_reason` |
| D26 | a good MPD keeps the parse failure count | `source/dash.rs` | killed | `a_good_mpd_between_bad_ones_resets_the_parse_failure_count` |
| D27 | parse reason not_mpd misspelt | `source/dash.rs` | killed | `an_mpd_that_is_not_xml_is_a_parse_error_reported_once_and_fatal_at_the_key_failure_limit`, `each_mpd_parse_failure_names_its_reason` |
| D28 | parse reason xml misspelt | `source/dash.rs` | killed | `a_good_mpd_after_failures_resets_the_failure_count`, `each_mpd_parse_failure_names_its_reason` |
| D29 | feature name mpd_patch misspelt | `source/dash.rs` | killed | `unsupported_features_are_reported_once_each_and_playback_goes_on` |
| D30 | feature name xlink misspelt | `source/dash.rs` | killed | `an_unresolved_xlink_set_is_reported_as_the_xlink_feature` |
| D31 | feature name utc_timing_scheme misspelt | `source/dash.rs` | killed | `utc_timing_notices_name_the_scheme_and_the_foreign_origin` |
| D32 | feature name utc_cross_origin misspelt | `source/dash.rs` | killed | `utc_timing_notices_name_the_scheme_and_the_foreign_origin` |
| D33 | features reported every time | `source/dash.rs` | killed | `a_feature_is_reported_once_per_session_even_across_a_reconnect` |
| D34 | FellBehind not mapped to LiveEdgeLost | `source/dash.rs` | killed | `falling_behind_the_window_reports_live_edge_lost_and_a_discontinuity_then_recovers`, `live_edge_lost_is_reported_again_after_the_session_recovered` |
| D35 | errors never re-armed by a success | `source/dash.rs` | killed | `live_edge_lost_is_reported_again_after_the_session_recovered` |
| D36 | Live announced with every sample batch | `source/dash.rs` | killed | `the_status_sequence_is_establishing_then_live_once` |
| D37 | Establishing announced with every response | `source/dash.rs` | killed | `the_status_sequence_is_establishing_then_live_once`, `the_variants_event_maps_the_manifest_and_names_the_active_audio` |
| D38 | samples of a stopped audio track still delivered | `source/dash.rs` | killed | `audio_can_be_stopped_and_started_again` |
| D39 | stopped audio still listed in the track set | `source/dash.rs` | killed | `audio_can_be_stopped_and_started_again` |
| D40 | manual video choice not re-applied in a later Period | `source/dash.rs` | killed | `a_manual_variant_choice_survives_a_period_change` |
| D41 | manual audio choice not re-applied in a later Period | `source/dash.rs` | killed | `a_manual_audio_choice_survives_a_period_change_by_name_and_language` |
| D42 | auto_select_audio=false still plays audio | `source/dash.rs` | killed | `auto_select_audio_off_plays_video_only_until_a_rendition_is_chosen` |
| D43 | AudioRendition announced every time | `source/dash.rs` | killed | `a_manual_audio_choice_survives_a_period_change_by_name_and_language` |
| D44 | unknown variant index accepted | `source/dash.rs` | killed | `an_unknown_variant_is_an_error_and_nothing_is_known_before_the_manifest` |
| D45 | reconnect does not announce a discontinuity | `source/dash.rs` | killed | `reconnect_restarts_a_static_presentation_from_a_fresh_mpd_after_a_discontinuity`, `reconnect_starts_a_fresh_presentation_and_keeps_the_host_clock` |
| D46 | reconnect drops the host clock | `source/dash.rs` | killed | `reconnect_starts_a_fresh_presentation_and_keeps_the_host_clock` |
| D47 | reconnect restarts an ended presentation | `source/dash.rs` | killed | `a_closed_session_ignores_everything_and_a_dash_session_reports_ended_once` |
| D48 | the host clock is not remembered | `source/dash.rs` | killed | `reconnect_starts_a_fresh_presentation_and_keeps_the_host_clock` |
| D49 | a static MPD gated as live | `source/dash.rs` | killed | `a_variant_switch_neither_restarts_nor_pauses_the_playout_gate`, `the_live_playout_lead_is_three_segments_at_most`, `the_vod_playout_gate_starts_at_the_vod_lead_and_the_live_gate_at_the_segment_based_lead` |
| D50 | live lead taken from a fixed one-second segment | `source/dash.rs` | killed | `the_live_playout_lead_is_three_segments_at_most` |
| D51 | retry due check off by one microsecond (resource) | `source/dash.rs` | killed | `a_hung_segment_is_closed_at_the_segment_timeout_and_requested_again`, `a_vod_segment_that_keeps_failing_is_fatal_after_the_configured_count_with_backoff`, `an_init_that_keeps_failing_is_fatal_init_fetch_failed_and_retry_after_is_honoured`, `retry_after_sets_the_wait_for_a_429_and_cookies_from_the_mpd_are_kept` |
| D52 | retry due check off by one microsecond (MPD) | `source/dash.rs` | killed | `an_mpd_retry_waits_for_retry_after`, `an_mpd_that_is_not_xml_is_a_parse_error_reported_once_and_fatal_at_the_key_failure_limit`, `live_number_presentation`, `live_timeline_presentation` |
| D53 | backoff ignores the attempt (resource) | `source/dash.rs` | killed | `a_vod_segment_that_keeps_failing_is_fatal_after_the_configured_count_with_backoff`, `retry_after_sets_the_wait_for_a_429_and_cookies_from_the_mpd_are_kept` |
| D54 | MPD retry ignores Retry-After | `source/dash.rs` | killed | `an_mpd_retry_waits_for_retry_after` |
| D55 | redirect target not passed to the core as the MPD base | `source/dash.rs` | killed | `an_mpd_redirect_moves_the_base_for_every_relative_reference` |
| D56 | response Date not passed on | `source/dash.rs` | killed | `live_number_presentation`, `live_timeline_presentation`, `the_date_header_is_the_last_clock_rung` |
| D57 | ETag not passed on | `source/dash.rs` | killed | `the_mpd_refresh_is_conditional_and_a_304_is_accepted` |
| D58 | UTC response Date not passed on | `source/dash.rs` | killed | `a_head_answer_declares_the_length_of_the_body_it_does_not_send`, `utc_head_is_sent_with_the_head_method_and_a_refused_head_is_retried_as_get` |
| D59 | request start not recorded | `source/dash.rs` | killed | `a_host_clock_step_does_not_disturb_a_utc_timing_anchored_session`, `a_hung_segment_is_closed_at_the_segment_timeout_and_requested_again`, `a_utc_request_is_abandoned_at_the_utc_timeout_and_an_init_at_the_request_timeout`, `every_addressing_joins_at_the_delay_and_plays_on_utc_timing` |
| D60 | response arrival not recorded | `source/dash.rs` | killed | `a_good_mpd_between_bad_ones_resets_the_parse_failure_count`, `a_head_answer_declares_the_length_of_the_body_it_does_not_send`, `a_host_clock_step_does_not_disturb_a_utc_timing_anchored_session`, `a_last_modified_validator_comes_back_as_if_modified_since_and_a_304_is_accepted` |
| D61 | a challenged request is not parked for credentials | `source/dash.rs` | killed | `a_401_waits_for_credentials_and_replays_the_parked_request` |
| D62 | parked requests not replayed on resume | `source/dash.rs` | killed | `a_401_waits_for_credentials_and_replays_the_parked_request` |
| D63 | a foreign origin's 401 prompts for credentials | `source/dash.rs` | killed | `a_401_from_another_origin_is_a_failed_request_not_a_credential_prompt` |
| D64 | protected-set drop not remembered | `source/dash.rs` | killed | `protected_content_is_reported_and_all_protected_content_is_fatal_unsupported_encryption` |
| D65 | undecodable init not remembered | `source/dash.rs` | killed | `an_undecodable_init_is_init_decode_failed_then_fatal_at_the_limit` |
| D66 | live segment loss reported without its status | `source/dash.rs` | killed | `a_live_segment_that_keeps_failing_is_skipped_with_one_error_and_a_discontinuity` |
| D67 | every drop reported separately | `source/dash.rs` | killed | `cross_origin_urls_off_refuses_a_base_url_on_another_origin`, `protected_content_is_reported_and_all_protected_content_is_fatal_unsupported_encryption` |
| D68 | all-skipped tracks not fatal | `source/dash.rs` | killed | `a_stream_whose_only_track_is_encrypted_is_fatal_unsupported_encryption` |
| D69 | encrypted sample entry not recognised | `source/dash.rs` | killed | `a_stream_whose_only_track_is_encrypted_is_fatal_unsupported_encryption`, `an_encrypted_track_is_reported_and_the_clear_track_still_plays` |
| D70 | non-fragmented MP4 reason misspelt | `source/dash.rs` | killed | `a_progressive_mp4_served_as_a_segment_is_a_fatal_unsupported_container` |
| E01 | core: stopping audio keeps requesting it | `dash-runtime: client/engine.rs` | killed | `stopping_audio_ends_its_requests_and_starting_it_again_resumes_from_the_next_segment` |
| E02 | core: restarting audio on the same set is a no-op | `dash-runtime: client/engine.rs` | killed | `stopping_audio_ends_its_requests_and_starting_it_again_resumes_from_the_next_segment` |
| E03 | core: a set plays its lowest-bandwidth Representation | `dash-runtime: client/engine.rs` | killed | `a_set_with_several_representations_plays_its_highest_bandwidth_one` |
| S01 | Auto never picks DASH for .mpd | `session.rs` | killed | `a_403_is_reported_and_the_request_is_retried`, `a_closed_session_ignores_everything_and_a_dash_session_reports_ended_once`, `a_feature_is_reported_once_per_session_even_across_a_reconnect`, `a_good_mpd_after_failures_resets_the_failure_count` |
| S02 | .mpd extension compared case-sensitively | `session.rs` | killed | `auto_picks_dash_for_an_mpd_path_in_any_case_with_or_without_a_query` |
| S03 | a DASH variant switch resets the playout gate | `session.rs` | killed | `a_variant_switch_neither_restarts_nor_pauses_the_playout_gate` |
| S04 | the playout gate is not engaged for DASH | `session.rs` | killed | `a_variant_switch_neither_restarts_nor_pauses_the_playout_gate`, `the_live_playout_lead_is_three_segments_at_most`, `the_vod_playout_gate_starts_at_the_vod_lead_and_the_live_gate_at_the_segment_based_lead` |
| S05 | set_wall_clock reaches a closed session | `session.rs` | killed | `a_clock_given_to_a_closed_session_produces_no_event` |
| S06 | set_wall_clock never reaches the DASH source | `session.rs` | killed | `a_utc_request_is_abandoned_at_the_utc_timeout_and_an_init_at_the_request_timeout`, `host_clock_number_template_joins_near_the_edge_and_plays_without_gaps`, `host_skew_without_utc_timing_is_absorbed_as_early_requests`, `prefer_host_takes_the_host_clock_before_utc_timing` |

## The survivor

- **D17** (`else if req.body.finish()` replaced by one that ignores its result): a `200` whole-file answer too short for the requested byte range is then delivered
  unsliced instead of failing in the adapter. The core's `Rejection::WrongLength` check rejects exactly that body, so the observable result (one counted
  failure, `Status::Reconnecting`, the retry) is identical: an equivalent mutant at the session boundary. The slicing rule itself (`BodyAcc::finish`) is
  pinned by `source::fetch::body`'s unit tests.
