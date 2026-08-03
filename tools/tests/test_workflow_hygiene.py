"""Structural checks on the GitHub workflow files.

Two of this repo's worst incidents were workflow-level, and neither produced a
red test:

1. A regex edit halved every indent in `ci.yml`. GitHub then failed the RUN
   before any job started and registered ZERO checks, so every PR showed green
   from the two surviving workflows while the entire gate suite never ran for
   ~12 hours. Nine PRs merged through that window.

2. The dep-consistency job shelled out to `cargo metadata` with no toolchain
   step at all. It died on a CalledProcessError before checking anything, so it
   had been "passing" by not executing.

Both are invisible to `cargo test`. These assertions are cheap and catch the
class.
"""

import pathlib

import pytest
import yaml

WORKFLOWS = sorted((pathlib.Path(__file__).parents[2] / ".github/workflows").glob("*.yml"))


def _load(path: pathlib.Path) -> dict:
    with path.open() as fh:
        return yaml.safe_load(fh)


def test_workflow_dir_is_not_empty() -> None:
    """Guard the guard: a bad glob here would make every test below vacuous."""
    assert len(WORKFLOWS) > 30, f"expected the full workflow set, found {len(WORKFLOWS)}"


@pytest.mark.parametrize("path", WORKFLOWS, ids=lambda p: p.name)
def test_workflow_parses_and_declares_jobs(path: pathlib.Path) -> None:
    """Invalid YAML fails the run before any job starts and registers NO checks
    — indistinguishable at a glance from a green PR."""
    doc = _load(path)
    assert doc is not None, f"{path.name} parsed as empty"
    assert doc.get("jobs"), f"{path.name} declares no jobs"
    for name, job in doc["jobs"].items():
        assert job.get("steps") or job.get("uses"), f"{path.name}:{name} has no steps"


@pytest.mark.parametrize("path", WORKFLOWS, ids=lambda p: p.name)
def test_workflow_has_a_trigger(path: pathlib.Path) -> None:
    """`on:` parses as the boolean True in YAML 1.1, so a missing trigger is
    easy to introduce and silent. One release workflow was once left as a bare
    `on: push:` firing on every push."""
    doc = _load(path)
    trigger = doc.get(True, doc.get("on"))
    assert trigger, f"{path.name} has no usable `on:` trigger"
