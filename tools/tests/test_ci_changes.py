import importlib.util
import pathlib

import pytest

SPEC = importlib.util.spec_from_file_location(
    "ci_changes", pathlib.Path(__file__).parents[1] / "ci_changes.py"
)
ci_changes = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci_changes)


@pytest.mark.parametrize(
    "paths",
    [
        ["docs/RELEASE-AUDIT.md"],
        ["CHANGELOG.md", "dvb-si/CHANGELOG.md", "docs/release-notes/v1.md"],
        ["CLAUDE.md"],
    ],
)
def test_markdown_only_skips_rust(paths):
    assert not ci_changes.rust_relevant(paths)


@pytest.mark.parametrize(
    "paths",
    [
        [],
        ["dvb-si/src/lib.rs"],
        ["dvb-si/README.md"],
        ["README.md"],
        ["docs/RELEASE-AUDIT.md", "dvb-si/src/lib.rs"],
        ["docs/enums/dvb/table_id.toml"],
        [".github/workflows/ci.yml"],
        ["Cargo.lock"],
        ["docs/RELEASE-AUDIT.md", "fixtures/ts/a.ts"],
    ],
)
def test_anything_else_runs_rust(paths):
    assert ci_changes.rust_relevant(paths)
