# Mutation table: the live `DashClient` (design step W-d2)

Each mutant is a one-line change to the core (`src/client/`, `src/timing/`, `src/address/`) applied on its own,
followed by `cargo nextest run -p dash-runtime --all-features --locked` over every test (the crate recompiled for each; the edit
was then reverted with `git checkout`). A mutant is **killed** when a test fails. `mutants_live.py` holds the edits
(`--dry` checks every pattern matches once).

Result: 54 of 60 mutants killed; 6 survive and are explained below the table.

| Id | DESIGN 8.4 row | File | Change | Result | Killed by |
|---|---|---|---|---|---|
| M01 | Availability | `timing/availability.rs` | `ticks_to_us(start_ticks.saturating_add(duration_ticks), time` -> `ticks_to_us(start_ticks, timescale),` | killed | closed_form_equals_brute_force_over_segment_formulas |
| M02 | Availability | `timing/availability.rs` | `.saturating_sub(ato_us) }` -> `.saturating_add(ato_us) }` | killed | availability_time_offset_makes_segments_available_earlier_only_when_applied, closed_form_equals_brute_force_over_segment_formulas |
| M03 | Availability | `timing/availability.rs` | `.saturating_add(tsb)     })` -> `.saturating_add(0)     })` | killed | public_formulas_on_hand_computed_numbers |
| M04 | Availability | `timing/live_edge.rs` | `s.available_from_us.is_some_and(\|a\| a <= utc_now_us)` -> `s.available_from_us.is_some_and(\|a\| a < utc_now_us)` | killed | livesim2_time_timeline_window_and_numbers |
| M05 | Availability | `timing/live_edge.rs` | `s.available_until_us.is_some_and(\|u\| u < utc_now_us)` -> `s.available_until_us.is_some_and(\|u\| u <= utc_now_us)` | killed | a_segment_is_in_the_window_through_its_availability_end_and_out_one_microsecond_later |
| M06 | Live edge | `timing/live_edge.rs` | `.saturating_sub(delay_us)         .saturating_sub(ast_us` -> `.saturating_add(delay_us)         .saturating_sub(ast_us` | killed | the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request, a_utc_server_behind_the_origin_joins_late_but_plays_cleanly |
| M07 | Live edge | `timing/live_edge.rs` | `pub const MIN_PRESENTATION_DELAY_US: i128 = 4 * MICROS_PER_S` -> `pub const MIN_PRESENTATION_DELAY_US: i128 = 0;` | killed | derived_presentation_delay_and_clamp |
| M08 | Live edge | `timing/live_edge.rs` | `Some(at.clamp(w.oldest, w.newest))` -> `Some(at)` | killed | closed_form_equals_brute_force_over_segment_formulas |
| M09 | Live edge | `address/plan.rs` | `Some(s) if s.start_us <= period_time_us => lo = mid + 1,` -> `Some(s) if s.start_us < period_time_us => lo = mid + 1,` | killed | livesim2_time_timeline_window_and_numbers |
| M10 | Clock | `timing/utc_timing.rs` | `seconds_us.saturating_add(half_second),` -> `seconds_us,` | killed | date_header_gets_the_half_second |
| M11 | Clock | `timing/utc_timing.rs` | `mono_us: exchange.sent_us + half,` -> `mono_us: exchange.sent_us.saturating_sub(half),` | killed | bodies_by_scheme, midpoint_and_uncertainty_follow_the_exchange |
| M12 | Clock | `timing/clock.rs` | `if policy.allow_downgrade {` -> `if true {` | killed | replacement_never_loses_the_most_trusted_anchor_under_random_offers, replacement_order |
| M13 | Clock | `timing/clock.rs` | `if diff > bars {` -> `if diff >= bars {` | killed | same_source_refines_only_when_at_least_as_precise_unless_stale_or_stepped |
| M14 | Clock | `timing/clock.rs` | `now_us.saturating_sub(old.mono_us) >= policy.resync_us` -> `now_us.saturating_sub(old.mono_us) > policy.resync_us` | killed | same_source_refines_only_when_at_least_as_precise_unless_stale_or_stepped |
| M15 | Refresh | `client/engine/live.rs` | `mup.map(\|m\| mid.saturating_add(m.max(floor)))` -> `mup.map(\|m\| mid.saturating_add(m))` | killed | minimum_update_period_zero_is_floored_at_one_second |
| M16 | Refresh | `client/engine/live.rs` | `(Some(n), Some(o)) if n < o` -> `(Some(n), Some(o)) if n > o` | killed | the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request |
| M17 | Refresh | `client/engine/live.rs` | `self.note_exchange(now_us, meta);         self.schedule_refr` -> `self.note_exchange(now_us, meta);         self.mpd_failed(no` | killed | an_unchanged_mpd_answers_304_and_the_conditional_headers_are_sent |
| M18 | Refresh | `client/engine/live.rs` | `validators = live.refresh.validators.clone();` -> `validators = MpdValidators::default();` | killed | an_unchanged_mpd_answers_304_and_the_conditional_headers_are_sent |
| M19 | Refresh | `client/engine/live.rs` | `(stale, live.refresh.body == bytes)` -> `(stale, false)` | killed | a_reset_descriptor_in_an_unchanged_mpd_resets_nothing |
| M20 | URLs | `client/engine/live.rs` | `live.refresh.redirect_url.clone()     }` -> `None     }` | killed | a_redirect_target_becomes_the_update_url_when_there_is_no_location |
| M21 | Errors | `client/engine/live.rs` | `\|\| from.is_some_and(\|f\| utc < f.saturating_add(grace));` -> `\|\| from.is_some_and(\|f\| utc < f);` | killed | a_404_inside_the_grace_window_is_early_and_after_it_is_missing |
| M22 | Errors | `client/engine/live.rs` | `status == Some(STATUS_TOO_EARLY)                 \|\| from` -> `false                 \|\| from` | killed | a_utc_server_ahead_of_the_origin_makes_early_requests_that_are_absorbed, a_425_is_never_counted_and_retried_at_250ms_doubling_to_one_second |
| M23 | Errors | `client/engine/live.rs` | `if until.is_some_and(\|u\| utc > u) {` -> `if false && until.is_some_and(\|u\| utc > u) {` | killed | a_response_that_arrives_after_its_segment_left_the_window_is_expired_not_counted |
| M24 | Errors | `client/engine.rs` | `if count >= self.max_failures(id) {` -> `if count > self.max_failures(id) {` | killed | a_cap_of_zero_or_one_makes_the_first_failure_final, a_permanent_init_fault_ends_after_the_cap_with_the_typed_fatal |
| M25 | Errors | `client/engine/live.rs` | `let delay = base.saturating_mul(1u64 << shift).min(cap);` -> `let delay = base.min(cap);` | killed | a_425_is_never_counted_and_retried_at_250ms_doubling_to_one_second |
| M26 | Errors | `client/engine/live.rs` | `let delay = base.saturating_mul(1u64 << shift).min(cap);` -> `let delay = base.saturating_mul(1u64 << shift);` | killed | a_425_is_never_counted_and_retried_at_250ms_doubling_to_one_second |
| M27 | Errors | `client/engine/live.rs` | `const EXPIRED_REJOIN_RUN: u32 = 3;` -> `const EXPIRED_REJOIN_RUN: u32 = 1_000_000;` | survived |  |
| M28 | Periods | `client/engine.rs` | `if !first && !continuous {             self.discontinuity();` -> `if !first && !continuous {}` | killed | continuity_naming_another_period_is_not_continuity, continuity_needs_the_same_adaptation_set_id |
| M29 | Periods | `client/engine.rs` | `SlotState::Marker => self.discontinuity(),` -> `SlotState::Marker => {}` | killed | a_switch_that_is_not_segment_aligned_is_marked_with_a_discontinuity |
| M30 | Availability | `client/engine/live.rs` | `if utc < from {                 return Candidate::Wait(from)` -> `if utc + 1_000_000 < from {                 return Candidate` | killed | the_ffmpeg_number_capture_plays_exactly_the_segments_the_clock_selects, the_ffmpeg_timeline_capture_joins_on_s_t_not_on_tfdt |
| M31 | Availability | `client/engine/live.rs` | `.unwrap_or(i128::MIN)                 .saturating_add(margin` -> `.unwrap_or(i128::MIN)                 .saturating_add(0);` | killed | the_availability_margin_delays_every_request_past_availability |
| M32 | Availability | `client/engine/live.rs` | `.with_availability_time_offset(self.dash.use_availability_ti` -> `.with_availability_time_offset(false);` | killed | use_availability_time_offset_requests_early_by_the_offset |
| M33 | Periods | `client/engine/live.rs` | `utc < ast.saturating_add(end).saturating_add(grace)` -> `false` | killed | expired_periods_dropping_out_of_the_mpd_do_not_disturb_playback, playing_through_two_boundaries_changes_period_twice_with_a_discontinuity_each |
| M34 | Errors | `client/engine.rs` | `self.outstanding < self.cfg.window()` -> `self.outstanding <= self.cfg.window()` | killed | completion_order_and_window_do_not_change_the_samples, the_window_fills_when_the_cap_allows_it |
| M35 | Memory | `client/engine.rs` | `.periods.first().is_some_and(\|p\| p.finished)` -> `.periods.first().is_some_and(\|_\| false)` | killed | twenty_four_hours_with_a_new_period_every_ten_minutes_keeps_every_collection_bounded |
| M36 | Memory | `client/engine.rs` | `while self.remembered_done.len() > MAX_REMEMBERED_DONE {` -> `while self.remembered_done.len() > usize::MAX {` | killed | twenty_four_hours_of_a_single_period_keeps_every_collection_bounded, twenty_four_hours_with_a_new_period_every_ten_minutes_keeps_every_collection_bounded |
| M37 | Refresh | `client/engine/live.rs` | `old.availability_start_time_us != new.availability_start_tim` -> `false` | killed | an_availability_start_change_is_an_mpd_reset_and_rejoins_the_live_edge |
| M38 | Clock | `client/engine/live.rs` | `let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptR` -> `let stepped = false && matches!(outcome, OfferOutcome::Adopt` | survived |  |
| M39 | Clock | `client/utc_sampler.rs` | `let head_refused = inflight.method == UtcMethod::Head` -> `let head_refused = false && inflight.method == UtcMethod::He` | killed | head_refused_with_405_is_reissued_as_get_and_still_anchors |
| M40 | Clock | `client/utc_sampler.rs` | `if !inflight.rtt_retried => {` -> `if true => {` | killed | a_utc_round_trip_over_the_limit_is_discarded_and_retried_once |
| M41 | Clock | `client/utc_sampler.rs` | `deadline_us: now_us.saturating_add(cfg.utc_timeout_us),` -> `deadline_us: now_us,` | killed | the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request, a_utc_round_trip_inside_the_limit_is_used_with_a_wider_error_bar |
| M42 | Clock | `client/engine/live.rs` | `sampler.wait_deadline_us = Some(now_us.saturating_add(self.d` -> `sampler.wait_deadline_us = Some(now_us);` | killed | the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request, a_utc_round_trip_inside_the_limit_is_used_with_a_wider_error_bar |
| M43 | Clock | `client/engine/live.rs` | `const FORCED_RESYNC_MIN_GAP_US: u64 = 30_000_000;` -> `const FORCED_RESYNC_MIN_GAP_US: u64 = 0;` | killed | randomised_sessions_never_panic_and_keep_their_invariants |
| M44 | Clock | `client/engine/live.rs` | `if self.clock.anchor().is_none() {             self.go_fatal` -> `if false {             self.go_fatal(FatalKind::ClockUnavail` | killed | a_dynamic_mpd_with_no_clock_source_is_fatal_and_nothing_else_is_requested, no_clock_source_at_all_is_a_typed_fatal_error |
| M45 | Clock | `client/engine/live.rs` | `let host_first = self.dash.clock_policy == DashClockPolicy::` -> `let host_first = false && self.dash.clock_policy == DashCloc` | killed | prefer_host_keeps_the_host_clock_through_a_forced_resync, prefer_host_uses_the_host_clock_and_never_asks_a_utc_server |
| M46 | Clock | `client/utc_sampler.rs` | `if let Some(url) = accept(v) {                         last.` -> `if let Some(url) = accept(v) {                         first` | killed | http_head_is_tried_after_the_other_http_schemes |
| M47 | Periods | `client/engine.rs` | `.ok_or(Error::NotSwitchable)?,             };` -> `.unwrap_or(0),             };` | survived |  |
| M48 | Errors | `client/engine/live.rs` | `if let Some((pi, li, _)) = self.rep_loc.get(&rep).copied()  ` -> `if let Some((pi, li, _)) = self.rep_loc.get(&rep).copied()  ` | survived |  |
| M49 | Periods | `client/engine/live.rs` | `&& m.media_presentation_duration.is_none()` -> `&& true` | killed | a_media_presentation_duration_ends_a_dynamic_presentation |
| M50 | Refresh | `client/engine/live.rs` | `self.force_refresh_after(now_us, wait);` -> `let _ = wait;` | killed | a_timeline_that_runs_out_before_the_next_refresh_forces_one |
| M51 | Clock | `client/engine/live.rs` | `ClockSource::UtcTiming,             now_us,             Offe` -> `ClockSource::Host,             now_us,             OfferPoli` | killed | a_utc_round_trip_inside_the_limit_is_used_with_a_wider_error_bar, a_utc_server_ahead_of_the_origin_makes_early_requests_that_are_absorbed |
| M52 | Refresh | `client/engine/live.rs` | `l.refresh.last_request_us = Some(meta.sent_us.min(meta.recei` -> `` | killed | a_timeline_that_runs_out_before_the_next_refresh_forces_one, minimum_update_period_zero_is_floored_at_one_second |
| M53 | Periods | `client/engine.rs` | `(exhausted && !self.period_open(pi, now_us))` -> `exhausted` | killed | the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request |
| M54 | Refresh | `client/engine/live.rs` | `if !self.mpd_inflight \|\| self.live.is_none() {             r` -> `if self.live.is_none() {             return Err(Error::MpdNo` | survived |  |
| M55 | Availability | `client/engine/live.rs` | `Some(a) if a > utc => Candidate::Wait(a),` -> `Some(a) if a >= utc => Candidate::Wait(a),` | survived |  |
| M56 | Errors | `client/engine/live.rs` | `if from.is_some_and(\|f\| utc >= f) {             self.request` -> `if false {             self.request_resync(now_us);` | killed | an_early_answer_the_clock_disagrees_with_asks_for_a_resync, early_answers_trigger_a_rate_limited_clock_resync_even_without_periodic_resync |
| M57 | Number | `address/plan.rs` | `let number = start_number.saturating_add(index);` -> `let number = start_number.saturating_add(index + 1);` | killed | extreme_inputs_never_panic_or_wrap |
| M58 | Timeline | `address/plan.rs` | `i128::from(s.time) - i128::from(self.pto),                  ` -> `i128::from(s.time),                     s.duration,         ` | killed | twenty_four_hours_with_a_new_period_every_ten_minutes_keeps_every_collection_bounded, playing_through_two_boundaries_changes_period_twice_with_a_discontinuity_each |
| M59 | Live edge | `timing/live_edge.rs` | `count.checked_sub(1) }` -> `Some(count) }` | killed | closed_form_equals_brute_force_over_segment_formulas, extreme_inputs_never_panic_or_wrap |
| M60 | Live edge | `client/engine/live.rs` | `Some(fetch.saturating_add(mup))` -> `None` | killed | a_trailing_negative_repeat_runs_to_the_fetch_time_plus_the_update_period |

Survivors:

- M27 (rejoin after 3 expiries becomes never) and M48 (the expiry run is not reset by a delivery): the effect is which segment a lane skips to after repeated expiry, which only a long adversarial schedule distinguishes; the single-expiry skip, the sleep and the late-response cases are tested.
- M38 (a `UTCTiming` step does not force a refresh): another forced refresh (an early or missing answer) fires in the same scenarios, so the extra trigger is not isolated by a black-box test.
- M47 (switching a Representation when the listed segments are used up): the fallback is exercised only when the lists are exhausted at the instant of the switch; the test hits it on some timings, not deterministically.
- M54 (`on_mpd_unchanged` without a request awaited): guarded twice (`live.is_none()`), the second guard makes the first redundant for a static client; a live client in that state is not reachable from the public API without a pending request.
- M55 (`a > utc` to `a >= utc` for the first-segment wake): equivalent; at equality the segment is already available and the lane is then evaluated as ready on the next pass.

Rows of DESIGN 8.4 not repeated here: `$Number$`, Timeline and URLs are in `mutation-table.md`, Containers and the Period/URL rows of the static client in `mutation-table-client.md`; the adapter-level `Retry-After` row belongs to the `broadcast-client` adapter (W-e).
