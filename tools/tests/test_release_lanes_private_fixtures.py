"""Every release lane runs `cargo test` as a publish gate, so it must be able to
run the private real-capture fixtures: each such job checks out the pinned
private commit through the shared composite action, and that action honours the
SKIP_PRIVATE_FIXTURES kill switch. A lane hand-patched once (buffering-runtime)
and every lane since copied nothing, so this is pinned structurally."""

import pathlib

import pytest
import yaml

REPO = pathlib.Path(__file__).parents[2]
WF = REPO / ".github" / "workflows"
ACTION = REPO / ".github" / "actions" / "private-fixtures" / "action.yml"
LANES = sorted(WF.glob("release*.yml"))
ACTION_USES = "./.github/actions/private-fixtures"
CHECKOUT = "actions/checkout@v5"


def _steps(path):
    doc = yaml.safe_load(path.read_text())
    for job, spec in doc.get("jobs", {}).items():
        yield job, spec.get("steps") or []


def _index(steps, pred):
    return [i for i, s in enumerate(steps) if isinstance(s, dict) and pred(s)]


def _runs_cargo_test(step):
    return "cargo test" in str(step.get("run", ""))


def _jobs_needing_private(who):
    for path in LANES:
        for job, steps in _steps(path):
            if _index(steps, _runs_cargo_test):
                yield pytest.param(path, steps, id=f"{who}/{path.name}::{job}")


CASES = list(_jobs_needing_private("lane"))


def test_lane_set_is_not_vacuous():
    assert ACTION.is_file(), f"{ACTION} is missing"
    assert len(LANES) == 52, [p.name for p in LANES]
    assert CASES, "no release job runs `cargo test`: the guard is vacuous"


@pytest.mark.parametrize("path,steps", CASES)
def test_cargo_test_job_checks_out_private_fixtures_after_checkout_and_before_test(path, steps):
    checkout = _index(steps, lambda s: str(s.get("uses")) == CHECKOUT)
    action = _index(steps, lambda s: s.get("uses") == ACTION_USES)
    test = _index(steps, _runs_cargo_test)
    assert checkout, f"{path.name}: no `{CHECKOUT}` step"
    assert len(action) == 1, f"{path.name}: expected one `{ACTION_USES}` step, found {len(action)}"
    assert checkout[0] < action[0] < test[0], (
        f"{path.name}: private-fixtures step at {action[0]} must come after the first checkout "
        f"({checkout[0]}) and before the first cargo test ({test[0]})"
    )


@pytest.mark.parametrize("path,steps", CASES)
def test_private_fixtures_step_passes_the_key_and_the_skip_switch(path, steps):
    (i,) = _index(steps, lambda s: s.get("uses") == ACTION_USES)
    with_ = steps[i].get("with") or {}
    assert with_.get("deploy-key") == "${{ secrets.PRIVATE_FIXTURES_DEPLOY_KEY }}", path.name
    assert with_.get("skip") == "${{ vars.SKIP_PRIVATE_FIXTURES }}", path.name


@pytest.mark.parametrize("path,steps", CASES)
def test_lanes_have_no_second_hand_rolled_private_checkout(path, steps):
    """The single source of truth: a copy-pasted step drifts from the action."""
    hand = [s for s in steps if isinstance(s, dict) and s.get("uses") == CHECKOUT and "repository" in (s.get("with") or {})]
    assert not hand, f"{path.name}: hand-rolled private checkout instead of `{ACTION_USES}`"
    assert "HAS_PRIVATE_KEY" not in path.read_text(), f"{path.name}: stale job-level HAS_PRIVATE_KEY"


def test_composite_action_shape():
    doc = yaml.safe_load(ACTION.read_text())
    assert doc["runs"]["using"] == "composite"
    for name in ("deploy-key", "skip"):
        assert name in doc["inputs"], f"action input `{name}` missing"
        assert doc["inputs"][name].get("required") is False, f"input `{name}` must be optional"
    assert str(doc["inputs"]["skip"].get("default")) in ("", "''", None)


def test_composite_action_honours_the_skip_switch():
    text = ACTION.read_text()
    doc = yaml.safe_load(text)
    assert "SKIP_PRIVATE_FIXTURES" in text and "REQUIRE_PRIVATE_FIXTURES=1" in text
    assert "secrets." not in text, "a composite action cannot read secrets: the key comes in as an input"
    for step in doc["runs"]["steps"]:
        if "echo \"RUST_BROADCAST_PRIVATE" in str(step.get("run", "")):
            break
    else:  # pragma: no cover - the assertion below is the failure
        pytest.fail("no step points RUST_BROADCAST_PRIVATE at the checkout")
    checkout = [s for s in doc["runs"]["steps"] if s.get("uses") == CHECKOUT]
    assert checkout, "the action never checks the private repo out"
    with_ = checkout[0].get("with") or {}
    assert with_.get("path") == "private-fixtures", "must not clobber the main checkout"
    assert with_.get("ssh-key") == "${{ inputs.deploy-key }}"
    assert "steps.private_pin.outputs.sha" in str(with_.get("ref")), "the ref must be the pinned commit"
