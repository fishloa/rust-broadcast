"""A complete CI run must clone the private fixtures repo (~374 MB) at most once.
GitHub jobs get separate runners, so that is only possible with one prep job
that saves the pinned checkout to the Actions cache and every consumer
restoring it. Before this, `test`, `test-no-default` and `dvb-stream` each
carried a copy-pasted deploy-key checkout — three clones a run. Pinned
structurally, because the copy-paste is exactly how it drifted."""

import pathlib

import pytest
import yaml

REPO = pathlib.Path(__file__).parents[2]
CI = REPO / ".github" / "workflows" / "ci.yml"
ACTION = REPO / ".github" / "actions" / "private-fixtures" / "action.yml"
ACTION_USES = "./.github/actions/private-fixtures"
PRIVATE_REPO = "fishloa/rust-broadcast-private"
PREP_JOB = "private-fixtures"
CHECKOUT = "actions/checkout@v5"
RESTORE = "actions/cache/restore@v4"
SAVE = "actions/cache/save@v4"
KEY = "${{ steps.pin.outputs.sha }}"


def _ci():
    return yaml.safe_load(CI.read_text())


def _action():
    return yaml.safe_load(ACTION.read_text())


def _mode(step):
    return (step.get("with") or {}).get("mode")


def _uses(step, uses):
    return isinstance(step, dict) and step.get("uses") == uses


def _private_checkouts(steps):
    return [
        i
        for i, s in enumerate(steps)
        if _uses(s, CHECKOUT) and PRIVATE_REPO in str((s.get("with") or {}).get("repository", ""))
    ]


def _consumers(doc):
    for job, spec in doc["jobs"].items():
        for step in spec.get("steps") or []:
            if _uses(step, ACTION_USES) and _mode(step) != "checkout":
                yield job, _mode(step), step


def test_ci_never_clones_the_private_repo_itself():
    doc = _ci()
    assert PRIVATE_REPO not in CI.read_text(), (
        f"{CI.name} must not name the private repo: the clone lives in {ACTION}"
    )
    for job, spec in doc["jobs"].items():
        assert not _private_checkouts(spec.get("steps") or []), f"{job}: hand-rolled private checkout"


def test_exactly_one_job_prepares_the_private_fixtures():
    prep = [job for job, mode, _ in _consumers(_ci()) if mode == "prepare"]
    assert prep == [PREP_JOB], f"expected exactly one prepare job ({PREP_JOB}), found {prep}"


def test_prepare_job_is_the_only_private_clone_and_needs_nothing():
    spec = _ci()["jobs"][PREP_JOB]
    assert "needs" not in spec, "the prep job must start immediately, not wait on another job"
    assert not spec.get("if"), "the prep job must never be conditionally skipped: consumers need it"
    steps = spec["steps"]
    (i,) = [i for i, s in enumerate(steps) if _uses(s, ACTION_USES)]
    assert _mode(steps[i]) == "prepare"
    with_ = steps[i]["with"]
    assert with_.get("deploy-key") == "${{ secrets.PRIVATE_FIXTURES_DEPLOY_KEY }}"
    assert with_.get("skip") == "${{ vars.SKIP_PRIVATE_FIXTURES }}"


def test_every_consumer_job_needs_the_prepare_job():
    doc = _ci()
    consumers = [job for job, mode, _ in _consumers(doc) if mode == "use"]
    assert sorted(consumers) == ["dvb-stream", "test", "test-no-default"], consumers
    for job in consumers:
        needs = doc["jobs"][job].get("needs")
        needs = [needs] if isinstance(needs, str) else list(needs or [])
        assert PREP_JOB in needs, f"{job}: mode `use` restores from the prep job, so it must need it"
        assert not doc["jobs"][job].get("if"), f"{job}: a skipped `use` job would silently skip the private tests"
        for step in doc["jobs"][job]["steps"]:
            if _uses(step, ACTION_USES):
                assert (step.get("with") or {}).get("deploy-key") == "${{ secrets.PRIVATE_FIXTURES_DEPLOY_KEY }}"
                assert (step.get("with") or {}).get("skip") == "${{ vars.SKIP_PRIVATE_FIXTURES }}"


def test_prepare_restores_the_cache_before_a_miss_conditional_checkout():
    steps = _action()["runs"]["steps"]
    restore = [s for s in steps if _uses(s, RESTORE)]
    save = [s for s in steps if _uses(s, SAVE)]
    checkout = _private_checkouts(steps)
    assert len(restore) == 1 and len(save) == 1 and len(checkout) == 1
    r, c, w = steps.index(restore[0]), checkout[0], steps.index(save[0])
    assert r < c < w, f"restore ({r}) -> checkout ({c}) -> save ({w})"
    assert "checkout" in str(restore[0].get("if")), "the lane checkout mode restores nothing"
    assert "cache-hit != 'true'" in str(steps[c].get("if")), "the clone must only run on a cache miss"
    for step in (restore[0], save[0]):
        with_ = step.get("with") or {}
        assert with_.get("path") == "private-fixtures"
        assert with_.get("key") == f"private-fixtures-v2-{KEY}"
    miss = str(restore[0]["with"].get("fail-on-cache-miss"))
    assert miss.startswith("${{") and miss.endswith("}}"), "must be an expression, not a literal string"
    assert "'use'" in miss, "only `use` fails on a miss; a prepare miss means clone it"
    assert (steps[c].get("with") or {}).get("ref") == KEY


def test_use_restore_fails_on_a_cache_miss():
    doc = _action()
    steps = doc["runs"]["steps"]
    (restore,) = [s for s in steps if _uses(s, RESTORE)]
    with_ = restore["with"]
    assert "checkout" in str(restore["if"])
    # actions/cache parses this input as a strict boolean: it MUST be a
    # `${{ }}` expression (a bare `inputs.mode == 'use'` is the literal string
    # and the step errors on every run).
    assert str(with_.get("fail-on-cache-miss")) == "${{ inputs.mode == 'use' }}", (
        "a `use` cache miss is a job-ordering bug and must fail, never silently skip"
    )
    assert with_.get("key") == f"private-fixtures-v2-{KEY}"
    assert with_.get("path") == "private-fixtures"
    assert "mode" in doc["inputs"] and doc["inputs"]["mode"].get("default") == "checkout"
    assert {"enabled", "sha"} <= set(doc["outputs"])


def test_use_mode_points_tests_at_the_restored_path():
    doc = _action()
    runs = [str(s["run"]) for s in doc["runs"]["steps"] if "run" in s]
    join = " ".join(runs)
    assert "RUST_BROADCAST_PRIVATE=$GITHUB_WORKSPACE/private-fixtures" in join
    assert "REQUIRE_PRIVATE_FIXTURES=1" in join
    assert "SKIP_PRIVATE_FIXTURES=1" in join, "a disabled use/checkout must set the kill switch explicitly"
    point = [s for s in doc["runs"]["steps"] if "RUST_BROADCAST_PRIVATE" in str(s.get("run", ""))]
    cond = str(point[0].get("if"))
    assert len(point) == 1 and "'use'" in cond and "'checkout'" in cond, cond


@pytest.mark.parametrize("name", ["SKIP_PRIVATE_FIXTURES", "REQUIRE_PRIVATE_FIXTURES", "private-fixtures"])
def test_skip_switch_is_still_honoured(name):
    assert name in ACTION.read_text()
