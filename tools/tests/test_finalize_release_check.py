"""tools/finalize-release-check.py: a consistent bumped tree passes; each single
defect flips exactly the check that names it (so no row can be vacuously green)."""

import importlib.util
import pathlib
import shutil
import subprocess

import pytest

SCRIPT = pathlib.Path(__file__).parents[1] / "finalize-release-check.py"
PAIRS = [("lib", "0.26.0"), ("app", "0.4.0")]
NOTE = "# t 1\n\n_Released 2026-10-16._\n\nbody\n"


def _load():
    spec = importlib.util.spec_from_file_location("finalize_release_check", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _crate(root, name, ver, deps=""):
    d = root / name
    (d / "src").mkdir(parents=True, exist_ok=True)
    (d / "src" / "lib.rs").write_text("")
    (d / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{ver}"\nedition = "2021"\n{deps}')
    (d / "CHANGELOG.md").write_text(f"# C\n\n## [Unreleased]\n\n## [{ver}] - 2026-10-17\n\n- x\n\n## [0.0.1] - 2020-01-01\n\n- old\n")


@pytest.fixture()
def tree(tmp_path):
    (tmp_path / "Cargo.toml").write_text('[workspace]\nmembers = ["lib", "app"]\nresolver = "2"\n')
    _crate(tmp_path, "lib", "0.26.0")
    _crate(tmp_path, "app", "0.4.0", '[dependencies]\nlib = { path = "../lib", version = "0.26" }\n')
    (tmp_path / "app" / "README.md").write_text('```toml\nlib = "0.26"\n```\n')
    notes = tmp_path / "docs" / "release-notes"
    notes.mkdir(parents=True)
    for c, v in PAIRS:
        (notes / f"{c}-{v}.md").write_text(NOTE)
    if shutil.which("cargo"):
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=tmp_path, check=True, capture_output=True)
    return tmp_path


def status(rows):
    return {name: s for name, s, _ in rows}


def test_consistent_tree_passes_everything(tree):
    fc = _load()
    rows = fc.run_checks(tree, PAIRS, skip_cargo=shutil.which("cargo") is None)
    st = status(rows)
    assert all(v in ("PASS", "INFO") for v in st.values()), rows
    assert st["manifest versions"] == st["epoch-pure siblings"] == st["release notes final"] == "PASS"


def _fail_set(rows):
    return {n for n, s, _ in rows if s == "FAIL"}


def test_wrong_manifest_version(tree):
    fc = _load()
    p = tree / "lib" / "Cargo.toml"
    p.write_text(p.read_text().replace("0.26.0", "0.25.0"))
    assert "manifest versions" in _fail_set(fc.run_checks(tree, PAIRS, skip_cargo=True))


def test_stale_epoch_and_bare_zero_requirement(tree):
    fc = _load()
    p = tree / "app" / "Cargo.toml"
    p.write_text(p.read_text().replace('version = "0.26"', 'version = "0.25"'))
    rows = fc.run_checks(tree, PAIRS, skip_cargo=True)
    assert "epoch-pure siblings" in _fail_set(rows)
    p.write_text(p.read_text().replace('version = "0.25"', 'version = "0"'))
    rows = fc.run_checks(tree, PAIRS, skip_cargo=True)
    assert "epoch-pure siblings" in _fail_set(rows)
    assert "spans every 0.x epoch" in dict((n, d) for n, _, d in rows)["epoch-pure siblings"]


def test_stale_epoch_in_dev_and_target_tables_is_caught(tree):
    fc = _load()
    p = tree / "app" / "Cargo.toml"
    p.write_text(p.read_text() + '[target.\'cfg(unix)\'.dev-dependencies]\nlib = { path = "../lib", version = "0.25" }\n')
    assert "epoch-pure siblings" in _fail_set(fc.run_checks(tree, PAIRS, skip_cargo=True))


def test_changelog_leftover_unreleased_and_missing_heading(tree):
    fc = _load()
    p = tree / "lib" / "CHANGELOG.md"
    p.write_text(p.read_text().replace("## [Unreleased]\n", "## [Unreleased]\n\n- forgotten\n"))
    rows = fc.run_checks(tree, PAIRS, skip_cargo=True)
    assert "CHANGELOG cut" in _fail_set(rows)
    p.write_text("# C\n\n## [Unreleased]\n")
    detail = dict((n, d) for n, _, d in fc.run_checks(tree, PAIRS, skip_cargo=True))["CHANGELOG cut"]
    assert "no `## [0.26.0]" in detail


def test_draft_note_missing_note_and_lockstep_note(tree):
    fc = _load()
    n = tree / "docs" / "release-notes"
    (n / "lib-0.26.0.md").write_text("---\nstatus: DRAFT\n---\n" + NOTE)
    assert "release notes final" in _fail_set(fc.run_checks(tree, PAIRS, skip_cargo=True))
    (n / "lib-0.26.0.md").write_text(NOTE)
    (n / "app-0.4.0.md").unlink()
    assert "release notes final" in _fail_set(fc.run_checks(tree, PAIRS, skip_cargo=True))


def test_lockstep_requires_all_five_identical(tree):
    fc = _load()
    for c in fc.LOCKSTEP:
        _crate(tree, c, "12.0.0")
        (tree / "docs" / "release-notes" / f"{c}-12.0.0.md").write_text(NOTE)
    (tree / "docs" / "release-notes" / "v12.0.0.md").write_text(NOTE)
    pairs = PAIRS + [(c, "12.0.0") for c in fc.LOCKSTEP]
    rows = fc.run_checks(tree, pairs, skip_cargo=True)
    assert status(rows)["lockstep five identical"] == "PASS"
    assert status(rows)["release notes final"] == "PASS"
    (tree / "docs" / "release-notes" / "v12.0.0.md").unlink()
    assert "release notes final" in _fail_set(fc.run_checks(tree, pairs, skip_cargo=True))
    (tree / "dvb-tools" / "Cargo.toml").write_text((tree / "dvb-tools" / "Cargo.toml").read_text().replace("12.0.0", "12.0.1"))
    rows = fc.run_checks(tree, pairs, skip_cargo=True)
    assert "lockstep five identical" in _fail_set(rows)


def test_readme_snippet_stale(tree):
    fc = _load()
    (tree / "app" / "README.md").write_text('```toml\nlib = "0.25"\n```\n')
    assert "README snippets" in _fail_set(fc.run_checks(tree, PAIRS, skip_cargo=True))


@pytest.mark.skipif(shutil.which("cargo") is None, reason="cargo not installed")
def test_stale_lock_fails_lock_rows(tree):
    fc = _load()
    lock = tree / "Cargo.lock"
    lock.write_text(lock.read_text().replace('name = "lib"\nversion = "0.26.0"', 'name = "lib"\nversion = "0.25.0"'))
    fails = _fail_set(fc.run_checks(tree, PAIRS))
    assert {"Cargo.lock versions", "cargo metadata --locked"} <= fails


def test_bindings_report_old_pins_without_failing(tree):
    fc = _load()
    b = tree / "bindings" / "python"
    b.mkdir(parents=True)
    (b / "Cargo.toml").write_text('[package]\nname = "py"\nversion = "0.1.0"\n[dependencies]\nlib = "0.20"\n')
    rows = fc.run_checks(tree, PAIRS, skip_cargo=True)
    row = [r for r in rows if r[0].startswith("bindings")][0]
    assert row[1] == "INFO" and "pins OLD" in row[2]
    (b / "Cargo.toml").write_text('[package]\nname = "py"\nversion = "0.1.0"\n[dependencies]\nlib = "0.26"\n')
    row = [r for r in fc.run_checks(tree, PAIRS, skip_cargo=True) if r[0].startswith("bindings")][0]
    assert "ALREADY on new epoch" in row[2]


def test_main_exit_code_and_pair_file(tree, capsys):
    fc = _load()
    pf = tree / "pairs"
    pf.write_text("lib=0.26.0\napp=0.4.0\n")
    assert fc.main(["x", str(pf), "--root", str(tree), "--skip-cargo"]) == 0
    (tree / "app" / "README.md").write_text('lib = "0.1"\n')
    assert fc.main(["x", str(pf), "--root", str(tree), "--skip-cargo"]) == 1
    assert "FAILED: README snippets" in capsys.readouterr().out
