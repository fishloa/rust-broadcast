"""Structure of ci.yml after the speed-up: markdown-only skipping must never
skip the always-on gates, the bare-feature lane must stay per-crate, and the
fuzz job must keep building every target on every shard."""

import pathlib

import yaml

REPO = pathlib.Path(__file__).parents[2]
CI = yaml.safe_load((REPO / ".github" / "workflows" / "ci.yml").read_text())
NIGHTLY = yaml.safe_load((REPO / ".github" / "workflows" / "nightly.yml").read_text())
JOBS = CI["jobs"]
SKIP = "needs.changes.outputs.rust == 'true'"
ALWAYS_ON = {"changes", "private-fixtures", "lint", "docs"}


def _needs(job):
    needs = JOBS[job].get("needs") or []
    return [needs] if isinstance(needs, str) else list(needs)


def _runs(job):
    return "\n".join(str(s.get("run", "")) for s in JOBS[job]["steps"])


def test_always_on_jobs_are_never_skipped():
    for job in ALWAYS_ON:
        assert not JOBS[job].get("if"), f"{job} must run on markdown-only changes too"


def test_every_other_job_skips_only_on_the_changes_gate():
    for job, spec in JOBS.items():
        if job in ALWAYS_ON:
            continue
        assert spec.get("if") == SKIP, f"{job}: must skip exactly on the changes gate"
        assert "changes" in _needs(job), f"{job}: reads the changes output"


def test_changes_job_uses_the_tested_classifier_and_main_runs_everything():
    run = _runs("changes")
    assert "tools/ci_changes.py" in run
    assert '"$EVENT" != "pull_request"' in run and "rust=true" in run


def test_no_default_lane_is_per_crate_not_a_workspace_rerun():
    run = _runs("test-no-default")
    assert "tools/check-bare-features.sh" in run
    assert "--workspace" in run  # the documented bare build gate stays
    assert "nextest run --workspace" not in run
    doc_lines = [l for l in run.splitlines() if "--doc" in l]
    assert doc_lines and all("--workspace" not in l for l in doc_lines)


def test_bare_registry_runs_do_not_unify_features():
    run = _runs("test-no-default")
    for crate in ("dvb-si", "dvb-t2mi"):
        assert f"nextest run -p {crate} --no-default-features --features std" in run


def test_fuzz_shards_cover_every_target_and_build_all():
    fuzz = JOBS["fuzz"]
    shards = fuzz["strategy"]["matrix"]["shard"]
    assert shards == list(range(len(shards)))
    run = _runs("fuzz")
    assert "fuzz build" in run, "every shard builds every target"
    assert f"tools/fuzz-smoke.sh ${{{{ matrix.shard }}}} {len(shards)} 10" in run


def test_dvb_stream_does_not_repeat_workspace_gates():
    run = _runs("dvb-stream")
    for dup in ("cargo fmt", "cargo clippy", "cargo doc"):
        assert dup not in run


def test_nightly_runs_soak_and_full_fuzz_budget():
    assert "schedule" in NIGHTLY[True] and "workflow_dispatch" in NIGHTLY[True]
    text = (REPO / ".github" / "workflows" / "nightly.yml").read_text()
    assert "RUST_BROADCAST_SOAK" in text
    assert "tools/fuzz-smoke.sh" in text and " 60" in text


def test_ci_cargo_profile_keeps_overflow_and_debug_assertions():
    import tomllib

    manifest = tomllib.loads((REPO / "Cargo.toml").read_text())
    ci = manifest["profile"]["ci"]
    assert ci["inherits"] == "test"
    assert ci["debug-assertions"] is True and ci["overflow-checks"] is True
    assert "opt-level" not in ci, "our own crates stay at opt-level 0"
    assert ci["package"]["*"]["opt-level"] == 2
    assert "dev" not in manifest["profile"], "local dev builds keep the default profile"


def test_test_jobs_run_under_the_ci_cargo_profile():
    for job in ("test", "dvb-stream"):
        assert "--cargo-profile ci" in _runs(job), job
    assert "--profile ci --doc" in _runs("test")
