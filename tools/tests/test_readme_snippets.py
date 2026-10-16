"""Self-test for tools/readme_snippets.py and its prepare-release.py wiring."""

import importlib.util
import pathlib

import pytest

TOOLS = pathlib.Path(__file__).parents[1]


def _load(name, file):
    spec = importlib.util.spec_from_file_location(name, TOOLS / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture()
def ws(tmp_path):
    for name, ver in (("lib", "0.25.0"), ("big", "11.1.0")):
        (tmp_path / name).mkdir()
        (tmp_path / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{ver}"\n')
    (tmp_path / "lib" / "README.md").write_text(
        "```toml\n"
        'lib = "0.24"\n'
        'lib = { version = "0.24", default-features = false }  # tight\n'
        'big = "11"\n'
        'serde = "1"\n'
        'lib = { path = "../lib" }\n'
        "```\n"
    )
    return tmp_path


def test_check_reports_every_stale_snippet_and_ignores_foreign_crates(ws):
    rs = _load("readme_snippets", "readme_snippets.py")
    bad = rs.check(ws, rs.workspace_versions(ws))
    assert len(bad) == 2, bad
    assert all("lib" in b and '"0.25"' in b for b in bad)


def test_planned_bump_changes_expectation_and_keeps_granularity(ws):
    rs = _load("readme_snippets", "readme_snippets.py")
    versions = rs.plan_versions([("lib", "0.26.0"), ("big", "12.0.0")], ws)
    changes = rs.rewrite(ws, versions)
    assert len(changes) == 3
    text = (ws / "lib" / "README.md").read_text()
    assert 'lib = "0.26"' in text
    assert 'lib = { version = "0.26", default-features = false }  # tight' in text
    assert 'big = "12"' in text  # major-only requirement stays major-only
    assert 'serde = "1"' in text and 'lib = { path = "../lib" }' in text
    assert rs.check(ws, versions) == []


def test_zero_x_never_shortened_below_major_minor():
    rs = _load("readme_snippets", "readme_snippets.py")
    assert rs.expected_req("0", "0.5.1") == "0.5"
    assert rs.expected_req("0.5.0", "0.5.1") == "0.5.1"
    assert rs.expected_req("3", "4.0.0") == "4"


def test_prepare_release_check_readmes_fails_then_apply_path_fixes(ws, monkeypatch, capsys):
    pr = _load("prepare_release", "prepare-release.py")
    monkeypatch.setattr(pr, "ROOT", ws)
    rs = pr._readme_snippets()
    monkeypatch.setattr(rs, "ROOT", ws)
    monkeypatch.setattr(pr, "_readme_snippets", lambda: rs)
    assert pr.check_readmes([]) == 1
    assert "stale README dependency snippet" in capsys.readouterr().err
    rs.rewrite(ws, rs.plan_versions([], ws))
    assert pr.check_readmes([]) == 0
    assert pr.check_readmes([("lib", "0.26.0")]) == 1  # planned bump disagrees
