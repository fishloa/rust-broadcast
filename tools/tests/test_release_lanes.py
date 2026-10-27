"""Structural pins on the 54 release lanes, the lockstep lane, the pair list and the
plan document: the audit found every one of these unpinned, and a drift in any of
them only shows up LIVE, mid-wave, on an immutable crates.io publish."""

import fnmatch
import importlib.util
import pathlib
import re
import subprocess
import sys

import pytest
import yaml

REPO = pathlib.Path(__file__).parents[2]
WF = REPO / ".github" / "workflows"
PAIRS_FILE = REPO / "docs" / "release-plan-2026-10.pairs"
LANES = sorted(WF.glob("release-*.yml"))
LOCKSTEP = ("dvb-si", "dvb-t2mi", "dvb-bbframe", "dvb-conformance", "dvb-tools")
ABANDONED = {"atsc3"}  # lane fails safe by design; never in the pair list
UNRELEASED: set[str] = {"tshttp-runtime", "broadcast-client", "dash-runtime"}  # lanes staged ahead of owner sign-off; leaves this set when a pairs file plans them


def _pr():
    spec = importlib.util.spec_from_file_location("prepare_release", REPO / "tools" / "prepare-release.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _glob(path):
    doc = yaml.safe_load(path.read_text())
    on = doc.get("on", doc.get(True))
    (g,) = on["push"]["tags"]
    return g


def _pairs():
    return [tuple(t.split("=", 1)) for t in PAIRS_FILE.read_text().split()]


def _planned():
    """Crates in the main plan or any follow-up patch plan (`release-plan-2026-10*.pairs`)."""
    out = set()
    for f in sorted((REPO / "docs").glob("release-plan-2026-10*.pairs")):
        out |= {t.split("=", 1)[0] for t in f.read_text().split()}
    return out


def test_lane_set_is_not_vacuous():
    assert len(LANES) == 54, [p.name for p in LANES]


@pytest.mark.parametrize("path", LANES, ids=lambda p: p.name)
def test_lane_glob_publish_package_and_note_path_name_the_same_crate(path):
    """tag glob `<crate>-v*`, `cargo publish -p <crate>`, the ref-name strip prefix
    and the release-note path must all agree: a lane copy-pasted from a sibling and
    only half-renamed publishes the WRONG crate (or gates the wrong note)."""
    glob = _glob(path)
    assert glob.endswith("-v*"), f"{path.name}: tag glob {glob!r} is not <crate>-v*"
    crate = glob[: -len("-v*")]
    text = path.read_text()
    assert (REPO / crate / "Cargo.toml").is_file(), f"{path.name}: no crate dir {crate}/"
    assert re.search(rf'^name\s*=\s*"{re.escape(crate)}"', (REPO / crate / "Cargo.toml").read_text(), re.M)
    published = set(re.findall(r"cargo publish -p ([\w-]+)", text))
    assert published == {crate}, f"{path.name}: publishes {published}, glob says {crate}"
    assert f"GITHUB_REF_NAME#{crate}-v" in text, f"{path.name}: version is not derived from the {crate}-v prefix"
    assert f"docs/release-notes/{crate}-" in text, f"{path.name}: release-note path does not name {crate}"
    assert "environment: crates-io" in text, f"{path.name}: publish is not behind the crates-io approval gate"
    for pkg in set(re.findall(r"cargo (?:test|clippy|build) -p ([\w-]+)", text)):
        assert pkg == crate, f"{path.name}: gate runs against {pkg}, not {crate}"


def test_every_plan_crate_has_exactly_one_lane_and_the_lockstep_has_none_of_its_own():
    lane_crates = {_glob(p)[: -len("-v*")] for p in LANES}
    planned = _planned()
    assert planned - set(LOCKSTEP) <= lane_crates, f"no lane for: {sorted(planned - set(LOCKSTEP) - lane_crates)}"
    assert not (lane_crates & set(LOCKSTEP)), "a lockstep crate has its own lane: it would publish twice"
    assert lane_crates - planned == ABANDONED | UNRELEASED, f"unplanned lanes: {sorted(lane_crates - planned - ABANDONED - UNRELEASED)}"
    assert set(LOCKSTEP) <= planned
    assert not (planned & ABANDONED)
    assert not (planned & UNRELEASED), "an unreleased crate must not be in the pair list"


def test_every_planned_tag_matches_exactly_one_lane_glob():
    """`dvb-ci-v*` must not also catch `dvb-ci-runtime-v...`, and `v*` must not
    catch a crate tag: two matching lanes would race to publish one version."""
    pairs = [f"{c}={v}" for c, v in _pairs()]
    tags = subprocess.run(
        [sys.executable, str(REPO / "tools" / "prepare-release.py"), "--tag-order", *pairs],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    globs = [_glob(p) for p in LANES] + [_glob(WF / "release.yml")]
    assert len(tags) == 50
    for t in tags:
        hits = [g for g in globs if fnmatch.fnmatchcase(t, g)]
        assert len(hits) == 1, f"tag {t} matches {hits}"


def test_lockstep_release_yml_publishes_in_dependency_order():
    text = (WF / "release.yml").read_text()
    order = re.findall(r"^\s*publish_crate ([\w-]+)\s*$", text, re.M)
    assert sorted(order) == sorted(LOCKSTEP), order
    pr = _pr()
    for i, crate in enumerate(order):
        deps = {d for d in pr._publish_deps(pr.crate_manifest(crate)) if d in LOCKSTEP and d != crate}
        later = deps - set(order[:i])
        assert not later, f"release.yml publishes {crate} before its lockstep dependency {sorted(later)}"
    gate = re.search(r"for c in ([\w\- ]+); do", text).group(1).split()
    assert sorted(gate) == sorted(LOCKSTEP), "the tag==version gate does not cover all five crates"


def test_lockstep_test_gate_has_a_timeout_and_matches_ci_flags():
    """The lockstep gate runs after ~33 crates are live: it must not hang forever."""
    doc = yaml.safe_load((WF / "release.yml").read_text())
    steps = {s.get("name"): s for s in doc["jobs"]["publish"]["steps"]}
    gate = steps["Gate — full test suite"]
    assert gate.get("timeout-minutes"), "no timeout on the lockstep test gate"
    assert "--locked" in gate["run"] and "--all-features" in gate["run"]


def test_yank_workflow_has_no_stale_default_crate_list():
    """yank.yml once defaulted `crates` to `dvb-common,dvb-si,...` (a crate that no longer
    exists, omitting every other): a bare `-f version=X` yanked the wrong set."""
    doc = yaml.safe_load((WF / "yank.yml").read_text())
    inputs = doc.get("on", doc.get(True))["workflow_dispatch"]["inputs"]
    assert inputs["crates"].get("required") is True and "default" not in inputs["crates"]
    assert "dvb-common" not in str(inputs)


def test_package_check_workflow_is_manual_only_and_runs_the_package_step():
    doc = yaml.safe_load((WF / "package-check.yml").read_text())
    on = doc.get("on", doc.get(True))
    assert list(on) == ["workflow_dispatch"], "package-check must stay manual (it is slow)"
    run = "\n".join(s.get("run", "") for s in doc["jobs"]["package-check"]["steps"])
    assert "tools/finalize-release-check.py" in run and "--package" in run
    assert "CARGO_REGISTRY_TOKEN" not in (WF / "package-check.yml").read_text(), "the package check must hold no publish secret"


def test_plan_section_6_is_the_generated_tag_order():
    pr = _pr()
    pairs = [f"{c}={v}" for c, v in _pairs()]
    order = subprocess.run(
        [sys.executable, str(REPO / "tools" / "prepare-release.py"), "--tag-order", *pairs],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    doc = (REPO / "docs" / "release-plan-2026-10.md").read_text()
    sec = doc[doc.index("## 6. Topological tag order") : doc.index("## 7. Release lanes")]
    listed = re.findall(r"^(\d+)\. `([^`]+)`", sec, re.M)
    assert [t for _, t in listed] == order, "plan section 6 drifted from `prepare-release.py --tag-order`"
    assert [int(n) for n, _ in listed] == list(range(1, len(order) + 1))
    generated = subprocess.run(
        [sys.executable, str(REPO / "tools" / "prepare-release.py"), "--tag-needs", *pairs],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    assert generated in sec, "plan section 6 is not the output of `--tag-needs` (regenerate it)"
    del pr


@pytest.mark.parametrize(
    "needle",
    [
        "git tag --points-at",  # not `git tag -l | wc -l`, which counts every tag in the repo
        "git ls-remote --tags origin",
        "|| { echo \"FAILED to create $t\"; break; }",  # the tag-creation loop checks its exit status
        "tools/check-release-env.py",
        "## 8. Failure playbook",
        "**(a) A lane gate failed",
        "**(b) A partial wave",
        "**(c) A partial lockstep",
        "12.0.1",
        "**(d) Yank vs patch-republish.**",
        "**(e) Rate limits and the three-tag rule.**",
        "Published-dependency consistency",
        "attach-binaries.yml -f tag=multimux-cli-v0.11.0",
        "zenith-tv",
        "axis-origin",
        "migration-notes-downstream.md",
        "--package",
        "--allow-tag-commit",
    ],
)
def test_runbook_carries_the_round_3_procedures(needle):
    assert needle in (REPO / "docs" / "RELEASE-RUNBOOK-2026-10.md").read_text()


def test_runbook_no_longer_counts_every_tag_in_the_repo():
    assert "git tag -l | wc -l" not in (REPO / "docs" / "RELEASE-RUNBOOK-2026-10.md").read_text()


def test_runbook_does_not_misplace_the_lockstep_slot():
    """The runbook once said v12.0.0 comes 'right after mpeg-ts'; the script's order
    puts it later. The doc may only say what the script does."""
    order = subprocess.run(
        [sys.executable, str(REPO / "tools" / "prepare-release.py"), "--tag-order", *[f"{c}={v}" for c, v in _pairs()]],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    assert order.index("v12.0.0") > order.index("mpeg-ts-v0.6.0") + 1  # the premise of the old claim is false
    runbook = (REPO / "docs" / "RELEASE-RUNBOOK-2026-10.md").read_text()
    assert "right after `mpeg-ts-v0.6.0`" not in runbook
    assert f"position {order.index('v12.0.0') + 1}" in runbook
