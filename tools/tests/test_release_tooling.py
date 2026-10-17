"""Tests for the mechanical release steps added to tools/prepare-release.py:
note finalisation, Cargo.lock refresh guard, crate-index step, tag order.

Each test is written so the behaviour it names can actually fail: the lock guard
is exercised with a real third-party mutation, the finaliser with a placeholder
hidden in a note that otherwise looks fine, the end-to-end run with a real cargo
workspace whose lock must change and must still resolve `--locked`.
"""

import importlib.util
import pathlib
import shutil
import subprocess

import pytest

SCRIPT = pathlib.Path(__file__).parents[1] / "prepare-release.py"


def _load():
    spec = importlib.util.spec_from_file_location("prepare_release", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture()
def pr(tmp_path, monkeypatch):
    mod = _load()
    monkeypatch.setattr(mod, "ROOT", tmp_path)
    return mod


GOOD = "---\nstatus: DRAFT\n---\n# lib 0.26.0\n\n_Released 2026-10-16._\n\nBody with a TODO(#12) mention mid-line.\n"


# ---------------------------------------------------------------- notes


def test_strip_keeps_body_byte_for_byte(pr):
    new, problem = pr.strip_draft_front_matter(GOOD)
    assert problem == ""
    assert new == "# lib 0.26.0\n\n_Released 2026-10-16._\n\nBody with a TODO(#12) mention mid-line.\n"


@pytest.mark.parametrize(
    "text, fragment",
    [
        ("# no front matter\n\nbody\n", "no front matter"),
        ("---\nstatus: DRAFT\n# never closed\n", "not closed"),
        ("---\nstatus: DRAFT\nowner: x\n---\n# t\n\nbody\n", "unexpected front matter"),
        ("---\nstatus: FINAL\n---\n# t\n\nbody\n", "unexpected front matter"),
        ("---\nstatus: DRAFT\n---\n# t\n\nTODO: write this\n", "placeholder"),
        ("---\nstatus: DRAFT\n---\n# t\n\n- TBD\n", "placeholder"),
        ("---\nstatus: DRAFT\n---\n# t\n\nsee [TODO] here\n", "placeholder"),
        ("---\nstatus: DRAFT\n---\n# t\n\nDRAFT notes\n", "placeholder"),
        ("---\nstatus: DRAFT\n---\n", "no body"),
    ],
)
def test_strip_fails_loudly(pr, text, fragment):
    new, problem = pr.strip_draft_front_matter(text)
    assert new is None
    assert fragment in problem


def test_finalize_writes_nothing_when_any_note_is_bad(pr):
    d = pr.ROOT / "docs" / "release-notes"
    d.mkdir(parents=True)
    (d / "a-1.0.0.md").write_text(GOOD)
    (d / "b-1.0.0.md").write_text("# b\n\nno front matter\n")
    log, problems = pr.finalize_notes([d / "a-1.0.0.md", d / "b-1.0.0.md", d / "c-1.0.0.md"], apply=True)
    assert len(problems) == 2 and any("MISSING" in p for p in problems)
    assert (d / "a-1.0.0.md").read_text() == GOOD  # the good note was NOT touched either


def test_finalize_applies_and_lists(pr):
    d = pr.ROOT / "docs" / "release-notes"
    d.mkdir(parents=True)
    (d / "a-1.0.0.md").write_text(GOOD)
    log, problems = pr.finalize_notes([d / "a-1.0.0.md"], apply=False)
    assert problems == [] and "a-1.0.0.md" in log[0]
    assert (d / "a-1.0.0.md").read_text() == GOOD  # dry run
    pr.finalize_notes([d / "a-1.0.0.md"], apply=True)
    assert not (d / "a-1.0.0.md").read_text().startswith("---")


def test_lockstep_note_included_and_five_must_agree(pr):
    five = [(c, "12.0.0") for c in pr.LOCKSTEP]
    names = [p.name for p in pr.notes_to_finalize(five, [])]
    assert "v12.0.0.md" in names and "dvb-si-12.0.0.md" in names
    assert pr.lockstep_version(five) == "12.0.0"
    assert pr.lockstep_version([("other", "0.1.0")]) is None
    with pytest.raises(SystemExit):
        pr.lockstep_version(five[:4])
    with pytest.raises(SystemExit):
        pr.lockstep_version(five[:4] + [("dvb-tools", "12.0.1")])


# ---------------------------------------------------------------- lock guard

LOCK_OLD = """version = 4

[[package]]
name = "app"
version = "0.3.0"
dependencies = [
 "lib",
 "serde 1.0.1",
]

[[package]]
name = "lib"
version = "0.25.0"

[[package]]
name = "serde"
version = "1.0.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aa"

[[package]]
name = "serde"
version = "1.0.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bb"
"""


def test_lock_guard_accepts_member_version_bumps_only(pr):
    new = LOCK_OLD.replace('name = "lib"\nversion = "0.25.0"', 'name = "lib"\nversion = "0.26.0"').replace(
        'name = "app"\nversion = "0.3.0"', 'name = "app"\nversion = "0.4.0"'
    )
    assert new != LOCK_OLD
    assert pr.lock_violations(LOCK_OLD, new) == []


def test_lock_guard_accepts_versioned_member_dep_string_change(pr):
    old = LOCK_OLD.replace(' "lib",', ' "lib 0.25.0",')
    new = LOCK_OLD.replace(' "lib",', ' "lib 0.26.0",').replace('name = "lib"\nversion = "0.25.0"', 'name = "lib"\nversion = "0.26.0"')
    assert pr.lock_violations(old, new) == []


@pytest.mark.parametrize(
    "mutate, fragment",
    [
        (lambda t: t.replace('version = "1.0.1"\nsource', 'version = "1.0.9"\nsource'), "serde@"),  # third-party float
        (lambda t: t.replace('checksum = "aa"', 'checksum = "zz"'), "serde@1.0.1"),  # same version, changed content
        (lambda t: t.replace(' "serde 1.0.1",', ' "serde 1.0.2",'), "app@member"),  # member now binds another serde
        (lambda t: t + '\n[[package]]\nname = "new"\nversion = "1.0.0"\nsource = "registry+x"\n', "added"),
        (lambda t: t.replace('[[package]]\nname = "serde"\nversion = "1.0.2"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "bb"\n', ""), "removed"),
    ],
)
def test_lock_guard_flags_everything_else(pr, mutate, fragment):
    new = mutate(LOCK_OLD)
    assert new != LOCK_OLD
    bad = pr.lock_violations(LOCK_OLD, new)
    assert bad and any(fragment in b for b in bad), bad


# ---------------------------------------------------------------- tag order


def _ws(root, spec):
    """spec: name -> (version, extra-toml)"""
    for name, (ver, extra) in spec.items():
        d = root / name
        (d / "src").mkdir(parents=True, exist_ok=True)
        (d / "src" / "lib.rs").write_text("")
        (d / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{ver}"\nedition = "2021"\n{extra}')


def test_tag_order_counts_versioned_dev_dependency_and_collapses_lockstep(pr):
    _ws(
        pr.ROOT,
        {
            "broadcast-common": ("10.0.0", ""),
            "mp4-emsg": ("0.5.0", '[dependencies]\nbroadcast-common = { path = "../broadcast-common", version = "10.0" }\n'),
            # mpeg-ts dev-depends (versioned) on mp4-emsg: must come AFTER it despite sorting before
            "aaa-ts": ("0.6.0", '[dependencies]\nbroadcast-common = { path = "../broadcast-common", version = "10.0" }\n[dev-dependencies]\nmp4-emsg = { path = "../mp4-emsg", version = "0.5" }\n'),
            # a path-only dev-dependency must NOT gate anything
            "zzz-path-only": ("0.1.0", '[dev-dependencies]\naaa-ts = { path = "../aaa-ts" }\n'),
            "dvb-si": ("12.0.0", '[dependencies]\naaa-ts = { path = "../aaa-ts", version = "0.6" }\n'),
            "dvb-t2mi": ("12.0.0", '[dependencies]\ndvb-si = { path = "../dvb-si", version = "12.0" }\n'),
            "dvb-bbframe": ("12.0.0", ""),
            "dvb-conformance": ("12.0.0", ""),
            "dvb-tools": ("12.0.0", ""),
            "consumer": ("0.1.0", '[dependencies]\ndvb-conformance = { path = "../dvb-conformance", version = "12.0" }\n'),
        },
    )
    pairs = [(c, v) for c, v in [("consumer", "0.1.0"), ("zzz-path-only", "0.1.0"), ("aaa-ts", "0.6.0"), ("mp4-emsg", "0.5.0"), ("broadcast-common", "10.0.0")]]
    pairs += [(c, "12.0.0") for c in pr.LOCKSTEP]
    order = pr.tag_order(pairs)
    assert order[0] == "broadcast-common-v10.0.0"
    assert order.count("v12.0.0") == 1 and not any(t.startswith("dvb-") for t in order)
    assert order.index("mp4-emsg-v0.5.0") < order.index("aaa-ts-v0.6.0") < order.index("v12.0.0") < order.index("consumer-v0.1.0")
    assert len(order) == len(set(order)) == 6


# ---------------------------------------------------------------- end to end


@pytest.mark.skipif(shutil.which("cargo") is None, reason="cargo not installed")
def test_apply_end_to_end_finalises_notes_refreshes_lock_and_check_is_read_only(pr, capsys):
    root = pr.ROOT
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["lib", "app"]\nresolver = "2"\n')
    _ws(root, {"lib": ("0.25.0", ""), "app": ("0.3.0", '[dependencies]\nlib = { path = "../lib", version = "0.25" }\n')})
    (root / "lib" / "CHANGELOG.md").write_text("# C\n\n## [Unreleased]\n\n### Added\n- y\n")
    (root / "app" / "CHANGELOG.md").write_text("# C\n\n## [Unreleased]\n\n## [0.3.0] - 2026-01-01\n- x\n")
    (root / "app" / "README.md").write_text('```toml\nlib = "0.25"\n```\n')
    notes = root / "docs" / "release-notes"
    notes.mkdir(parents=True)
    (notes / "lib-0.26.0.md").write_text(GOOD)
    (notes / "app-0.4.0.md").write_text(GOOD)
    (root / "tools").mkdir()
    (root / "tools" / "gen-crate-index.py").write_text("import sys\nprint('gen ok', sys.argv[1:])\n")
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
    lock_before = (root / "Cargo.lock").read_text()
    assert 'name = "lib"\nversion = "0.25.0"' in lock_before

    snapshot = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
    assert pr.main(["x", "--check", "lib=0.26.0", "app=0.4.0"]) == 0
    out = capsys.readouterr().out
    assert "would run: cargo metadata" in out and "would run: python3 tools/gen-crate-index.py" in out
    assert "lib-0.26.0.md: finalised" in out
    assert snapshot == {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}, "--check wrote something"

    assert pr.main(["x", "lib=0.26.0", "app=0.4.0"]) == 0
    out = capsys.readouterr().out
    assert "gen ok" in out and "Cargo.lock refreshed: 2 line(s) changed" in out
    lock_after = (root / "Cargo.lock").read_text()
    assert 'name = "lib"\nversion = "0.26.0"' in lock_after and 'name = "app"\nversion = "0.4.0"' in lock_after
    assert not (notes / "lib-0.26.0.md").read_text().startswith("---")
    assert 'lib = "0.26"' in (root / "app" / "README.md").read_text()
    locked = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"], cwd=root, capture_output=True)
    assert locked.returncode == 0


def test_lock_guard_member_set_change_is_only_tolerated_when_asked(pr):
    new = LOCK_OLD + '\n[[package]]\nname = "newmember"\nversion = "0.1.0"\n'
    assert any("added to Cargo.lock: newmember@member" in b for b in pr.lock_violations(LOCK_OLD, new))
    assert pr.lock_violations(LOCK_OLD, new, allow_member_set_change=True) == []
    # ...but a third-party change is never tolerated, even with the flag
    bad = LOCK_OLD.replace('checksum = "aa"', 'checksum = "zz"')
    assert pr.lock_violations(LOCK_OLD, bad, allow_member_set_change=True)


@pytest.mark.skipif(shutil.which("cargo") is None, reason="cargo not installed")
def test_demo_lock_is_refreshed_with_the_guard(pr, capsys):
    """demo/ is its own workspace with a committed lock recording member versions."""
    root = pr.ROOT
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["lib"]\nexclude = ["demo"]\nresolver = "2"\n')
    _ws(root, {"lib": ("0.25.0", "")})
    demo = root / "demo"
    (demo / "src").mkdir(parents=True)
    (demo / "src" / "lib.rs").write_text("")
    (demo / "Cargo.toml").write_text(
        '[package]\nname = "dvb-demo"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n\n'
        '[dependencies]\nlib = { path = "../lib", version = "0.25" }\n'
    )
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=demo, check=True, capture_output=True)
    assert 'name = "lib"\nversion = "0.25.0"' in (demo / "Cargo.lock").read_text()
    # the release bump
    (root / "lib" / "Cargo.toml").write_text((root / "lib" / "Cargo.toml").read_text().replace("0.25.0", "0.26.0"))
    (demo / "Cargo.toml").write_text((demo / "Cargo.toml").read_text().replace('version = "0.25"', 'version = "0.26"'))

    assert pr.refresh_demo_lock(False) == 0 and "would run: cargo metadata" in capsys.readouterr().out
    assert 'name = "lib"\nversion = "0.25.0"' in (demo / "Cargo.lock").read_text(), "dry run wrote the lock"
    assert pr.refresh_demo_lock(True) == 0
    assert 'name = "lib"\nversion = "0.26.0"' in (demo / "Cargo.lock").read_text()
    assert "demo/Cargo.lock refreshed" in capsys.readouterr().out
    locked = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--manifest-path", "demo/Cargo.toml"], cwd=root, capture_output=True)
    assert locked.returncode == 0


def test_demo_lock_absent_is_a_noop(pr, capsys):
    assert pr.refresh_demo_lock(True) == 0
    assert "absent" in capsys.readouterr().out


REAL_ROOT = pathlib.Path(__file__).parents[2]


def test_manifests_gap_is_harmless_spikes_and_differential_sweep_carry_no_versioned_sibling_requirement():
    """prepare-release.py's manifests() does not scan spikes/ or tools/differential-sweep.
    That is only safe while neither carries a *versioned* requirement on a workspace
    member (path-only deps have nothing to bump). If one gains one, this fails and
    manifests() must grow."""
    import tomllib

    members = {p.parent.name for p in REAL_ROOT.glob("*/Cargo.toml")}
    for rel in ("spikes/trunk-bench/Cargo.toml", "tools/differential-sweep/Cargo.toml"):
        data = tomllib.loads((REAL_ROOT / rel).read_text())
        tables = [data] + list((data.get("target") or {}).values())
        for t in tables:
            for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                for name, spec in (t.get(kind) or {}).items():
                    real = spec.get("package", name) if isinstance(spec, dict) else name
                    if real in members:
                        assert isinstance(spec, dict) and "version" not in spec, f"{rel}: {name} carries a version requirement"


def test_tag_needs_lists_direct_dependencies_and_the_lockstep_slot_first(pr):
    _ws(
        pr.ROOT,
        {
            "broadcast-common": ("10.0.0", ""),
            "dvb-si": ("12.0.0", '[dependencies]\nbroadcast-common = { path = "../broadcast-common", version = "10.0" }\n'),
            "dvb-t2mi": ("12.0.0", '[dependencies]\ndvb-si = { path = "../dvb-si", version = "12.0" }\n'),
            "dvb-bbframe": ("12.0.0", ""),
            "dvb-conformance": ("12.0.0", ""),
            "dvb-tools": ("12.0.0", ""),
            "consumer": ("0.1.0", '[dependencies]\ndvb-conformance = { path = "../dvb-conformance", version = "12.0" }\nbroadcast-common = { path = "../broadcast-common", version = "10.0" }\n'),
        },
    )
    pairs = [("consumer", "0.1.0"), ("broadcast-common", "10.0.0")] + [(c, "12.0.0") for c in pr.LOCKSTEP]
    needs = pr.tag_needs(pairs)
    assert needs["broadcast-common-v10.0.0"] == []
    assert needs["v12.0.0"] == ["broadcast-common"]  # the intra-lockstep edge dvb-t2mi -> dvb-si is not an external need
    assert needs["consumer-v0.1.0"] == ["v12.0.0", "broadcast-common"]
    md = pr.tag_needs_markdown(pairs)
    assert md.splitlines()[0] == "1. `broadcast-common-v10.0.0`  " and "needs live first: (nothing in this wave)" in md


def test_preflight_aborts_before_editing_anything(pr, capsys):
    root = pr.ROOT
    _ws(root, {"lib": ("0.25.0", "")})
    (root / "lib" / "CHANGELOG.md").write_text("# C\n\n## [Unreleased]\n\n- y\n")
    (root / "docs" / "release-notes").mkdir(parents=True)
    (root / "docs" / "release-notes" / "lib-0.26.0.md").write_text("# lib\n\nTODO write me\n")
    before = (root / "lib" / "Cargo.toml").read_text()
    assert pr.main(["x", "lib=0.26.0"]) == 1
    assert (root / "lib" / "Cargo.toml").read_text() == before
    assert "no front matter" in capsys.readouterr().err


# ---------------------------------------------------------------- CI wiring

WORKFLOWS = pathlib.Path(__file__).parents[2] / ".github" / "workflows"


def test_ci_runs_both_generated_doc_gates():
    import yaml

    doc = yaml.safe_load((WORKFLOWS / "ci.yml").read_text())
    runs = [s.get("run", "") for j in doc["jobs"].values() for s in j.get("steps", [])]
    assert "python3 tools/readme_snippets.py --check" in runs
    assert "python3 tools/gen-crate-index.py --check" in runs


def test_dep_consistency_is_blocking_on_release_tags():
    import yaml

    doc = yaml.safe_load((WORKFLOWS / "published-dep-consistency.yml").read_text())
    steps = doc["jobs"]["check"]["steps"]
    blocking = [s for s in steps if s.get("run", "").endswith("--blocking")]
    assert len(blocking) == 1 and "refs/tags/" in blocking[0]["if"]
    trig = doc.get(True, doc.get("on"))
    assert "*-v*" in trig["push"]["tags"] and "v*" in trig["push"]["tags"]
