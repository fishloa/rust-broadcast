"""Guard tests for tools/check-citations.py."""

import importlib.util
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("check_citations", ROOT / "tools" / "check-citations.py")
cc = importlib.util.module_from_spec(SPEC)
sys.modules["check_citations"] = cc
SPEC.loader.exec_module(cc)

_DOCS = "docs"


def _scratch(tmp_path: pathlib.Path, files: dict[str, str]) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    for rel, text in files.items():
        p = tmp_path / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)


def _run(monkeypatch, tmp_path: pathlib.Path, env=None) -> int:
    monkeypatch.setattr(cc, "ROOT", tmp_path)
    if env:
        for k, v in env.items():
            monkeypatch.setenv(k, v)
    return cc.main()


def test_a_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {_DOCS + "/good.md": "see " + _DOCS + "/missing.md\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_a_passes(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {_DOCS + "/good.md": "see " + _DOCS + "/exists.md\n", _DOCS + "/exists.md": "hi\n"})
    assert _run(monkeypatch, tmp_path) == 0


def test_b_fails(tmp_path, monkeypatch) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    _scratch(tmp_path, {"good.md": "see private/missing.md\n", "private/specs-verified/.keep": ""})
    # ensure private root present
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    # re-add
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)
    assert _run(monkeypatch, tmp_path) == 1


def test_b_passes(tmp_path, monkeypatch) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "exists.md").parent.mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "exists.md").write_text("hi\n", encoding="utf-8")
    _scratch(tmp_path, {"good.md": "see private/exists.md\n"})
    # place private file inside tmp tree (not tracked? make tracked)
    (tmp_path / "private" / "exists.md").write_text("hi\n", encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)
    assert _run(monkeypatch, tmp_path) == 0


def test_c_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"good.md": "see [x](private/secret.md)\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_c_passes(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"good.md": "see [x](exists.md)\n", "exists.md": "hi\n"})
    assert _run(monkeypatch, tmp_path) == 0



_URL = "https://github.com/fishloa/rust-broadcast/blob/main/"


def test_d_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"good.md": "see " + _URL + "missing.md\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_d_passes(tmp_path, monkeypatch) -> None:
    (tmp_path / "good.md").write_text("hi\n", encoding="utf-8")
    _scratch(tmp_path, {"good.md": "see " + _URL + "good.md\n"})
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)
    assert _run(monkeypatch, tmp_path) == 0



def test_private_citation_passes_check_a_when_absent(tmp_path, monkeypatch) -> None:
    # private/ tree absent; a private/docs/... citation must not be flagged by check a
    _scratch(tmp_path, {"good.md": "see private/docs/x.md\n"})
    # ensure private root absent
    assert not (tmp_path / "private" / "specs-verified").exists()
    assert _run(monkeypatch, tmp_path) == 0


def test_github_url_into_private_fails(tmp_path, monkeypatch) -> None:
    (tmp_path / "private").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "secret.md").write_text("hi\n", encoding="utf-8")
    _scratch(tmp_path, {"good.md": "see " + _URL + "private/secret.md\n"})
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)
    assert _run(monkeypatch, tmp_path) == 1
def test_committed_tree_passes() -> None:
    assert cc.main() == 0


def test_fixture_ts_missing_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"docs/good.md": "see fixtures/ts/missing.ts\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_fixture_ts_existing_passes(tmp_path, monkeypatch) -> None:
    (tmp_path / "fixtures" / "ts").mkdir(parents=True, exist_ok=True)
    (tmp_path / "fixtures" / "ts" / "exists.ts").write_text("x\n")
    _scratch(tmp_path, {"docs/good.md": "see fixtures/ts/exists.ts\n", "fixtures/ts/exists.ts": "x\n"})
    assert _run(monkeypatch, tmp_path) == 0


def test_fixture_private_mp4_missing_fails_when_present(tmp_path, monkeypatch) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    _scratch(tmp_path, {"good.md": "see private/fixtures/x/missing.mp4\n"})
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)
    assert _run(monkeypatch, tmp_path) == 1


def test_fixture_private_mp4_missing_passes_when_absent(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"good.md": "see private/fixtures/x/missing.mp4\n"})
    assert not (tmp_path / "private" / "specs-verified").exists()
    assert _run(monkeypatch, tmp_path) == 0


def test_crate_tests_fixtures_fails(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"dvb-si"})
    _scratch(tmp_path, {"docs/good.md": "see dvb-si/tests/fixtures/x.ts\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_crate_fixtures_fails(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"dvb-si"})
    _scratch(tmp_path, {"docs/good.md": "see dvb-si/fixtures/x.bin\n"})
    assert _run(monkeypatch, tmp_path) == 1
