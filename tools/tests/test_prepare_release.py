"""Self-test for tools/prepare-release.py against a throwaway workspace.

Covers the two gaps found in the 2026-10-13 wave:
1. an EMPTY `## [Unreleased]` must still cut a dated section + scaffold a note;
2. a sibling dependency written in a SECOND table (dev-/build-/target) must be
   updated even when the first entry in the file is already current.
"""

import importlib.util
import pathlib
import shutil
import subprocess

import pytest

SCRIPT = pathlib.Path(__file__).parents[1] / "prepare-release.py"


@pytest.fixture()
def pr(tmp_path, monkeypatch):
    spec = importlib.util.spec_from_file_location("prepare_release", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    monkeypatch.setattr(mod, "ROOT", tmp_path)

    (tmp_path / "Cargo.toml").write_text('[workspace]\nmembers = ["lib", "app"]\nresolver = "2"\n')
    for name, ver in (("lib", "0.25.0"), ("app", "0.3.0")):
        d = tmp_path / name
        (d / "src").mkdir(parents=True)
        (d / "src" / "lib.rs").write_text("")
    (tmp_path / "lib" / "Cargo.toml").write_text('[package]\nname = "lib"\nversion = "0.25.0"\nedition = "2021"\n')
    (tmp_path / "app" / "Cargo.toml").write_text(
        '[package]\nname = "app"\nversion = "0.3.0"\nedition = "2021"\n\n'
        '[dependencies]\nlib = { path = "../lib", version = "0.26" }\n\n'
        '[dev-dependencies]\nlib = { path = "../lib", version = "0.25", default-features = false, features = ["cli"] }\n\n'
        '[target.\'cfg(unix)\'.dependencies]\nlib = { path = "../lib", version = "0.25" }\n'
    )
    # app's CHANGELOG has an EMPTY [Unreleased]; lib's has content.
    (tmp_path / "app" / "CHANGELOG.md").write_text("# Changelog\n\n## [Unreleased]\n\n## [0.3.0] - 2026-01-01\n### Added\n- x\n")
    (tmp_path / "lib" / "CHANGELOG.md").write_text("# Changelog\n\n## [Unreleased]\n\n### Added\n- y\n")
    return mod


def test_every_table_entry_updated_even_if_first_is_current(pr):
    # First entry already says 0.26, the later ones say 0.25 -- the old code
    # looked only at the first match and skipped the file.
    lines = pr.update_sibling_refs("lib", "0.26.0", apply=True)
    assert len(lines) == 2, lines
    text = (pr.ROOT / "app" / "Cargo.toml").read_text()
    assert text.count('version = "0.26"') == 3
    assert '"0.25"' not in text
    assert 'features = ["cli"]' in text  # rest of the entry untouched


def test_empty_unreleased_still_cuts_section_and_note(pr):
    status, body = pr.cut_changelog("app", "0.3.1", True, "Dependency/epoch bump only (follows lib); no functional change.")
    assert "EMPTY" in status
    text = (pr.ROOT / "app" / "CHANGELOG.md").read_text()
    assert f"## [0.3.1] - {pr.TODAY}" in text
    assert "### Changed\n- Dependency/epoch bump only (follows lib)" in text
    assert text.index("## [Unreleased]") < text.index("## [0.3.1]") < text.index("## [0.3.0]")
    assert body
    msg = pr.scaffold_release_note("app", "0.3.1", body, True)
    assert "scaffolded" in msg
    assert (pr.ROOT / "docs" / "release-notes" / "app-0.3.1.md").is_file()


def test_default_note_names_sibling_when_depending_on_bumped_crate(pr):
    assert pr.depends_on_any("app", ["lib", "app"]) == ["lib"]
    assert pr.depends_on_any("lib", ["lib", "app"]) == []


@pytest.mark.skipif(shutil.which("cargo") is None, reason="cargo not installed")
def test_resolution_check_reports_failure(pr, capsys):
    root = pr.ROOT
    # app wants lib 0.99 -> unsatisfiable: must be reported, not silent.
    (root / "app" / "Cargo.toml").write_text(
        '[package]\nname = "app"\nversion = "0.3.0"\nedition = "2021"\n\n'
        '[dependencies]\nlib = { path = "../lib", version = "0.99" }\n'
    )
    assert pr.verify_resolution(root) == 1
    assert "ERROR" in capsys.readouterr().err
    pr.update_sibling_refs("lib", "0.25.0", apply=True)
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
    assert pr.verify_resolution(root) == 0
