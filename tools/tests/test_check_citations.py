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


def test_fixture_dir_wrong_case_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"docs/good.md": "see fixtures/scte35/V300/\n", "fixtures/scte35/v300/a.mpd": "x\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_fixture_dir_exact_case_passes(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"docs/good.md": "see fixtures/scte35/v300/\n", "fixtures/scte35/v300/a.mpd": "x\n"})
    assert _run(monkeypatch, tmp_path) == 0


def _errors(monkeypatch, tmp_path: pathlib.Path, capsys) -> str:
    rc = _run(monkeypatch, tmp_path)
    err = capsys.readouterr().err
    assert rc == (1 if err else 0)
    return err


def test_retired_crate_tests_fixtures_path_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"docs/good.md": "see `tests/fixtures/x.ts`\n"})
    assert "retired tests/fixtures or tests/golden path" in _errors(monkeypatch, tmp_path, capsys)


def test_retired_crate_tests_golden_path_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"docs/good.md": "bless into multimux/tests/golden\n"})
    assert "retired tests/fixtures or tests/golden path" in _errors(monkeypatch, tmp_path, capsys)


def test_retired_manifest_exclude_pattern_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/Cargo.toml": 'exclude = ["docs/**", "tests/fixtures/**"]\n'})
    assert "retired tests/fixtures or tests/golden path" in _errors(monkeypatch, tmp_path, capsys)


def test_golden_gate_file_name_is_not_a_retired_path(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"transmux"})
    _scratch(
        tmp_path,
        {
            "docs/good.md": "see transmux/tests/golden_gate.rs and transmux/tests/golden.rs and tests/golden_wire\n",
            "transmux/tests/golden_gate.rs": "\n",
            "transmux/tests/golden.rs": "\n",
        },
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_retired_path_in_exempt_guard_file_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"docs/FIXTURES.md": "no tracked file may match `*/tests/golden/*`\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_retired_path_in_changelog_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/CHANGELOG.md": "moved tests/fixtures/x.ts\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_retired_docs_specs_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"docs/good.md": "transcribe into `docs/specs/`\n"})
    assert "retired docs/specs/ tree" in _errors(monkeypatch, tmp_path, capsys)


def test_retired_private_map_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"docs/good.md": "see PRIVATE-MAP for the layout\n"})
    assert "PRIVATE-MAP" in _errors(monkeypatch, tmp_path, capsys)


def test_retired_test_streams_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/src/lib.rs": "// reads .test-streams/x.ts\n"})
    assert ".test-streams" in _errors(monkeypatch, tmp_path, capsys)


def test_test_streams_allowed_in_fetch_script(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"tools/fetch-test-streams.sh": 'DEST="$ROOT/.test-streams"\n'})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_relative_fixture_citation_missing_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/tests/it/a.rs": "//! reads `../fixtures/ts/missing.ts`\n"})
    assert "relative fixture citation does not resolve: ../fixtures/ts/missing.ts" in _errors(
        monkeypatch, tmp_path, capsys
    )


def test_relative_fixture_citation_existing_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {"foo/tests/it/a.rs": "//! reads `../fixtures/ts/here.ts`\n", "fixtures/ts/here.ts": "x\n"},
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_relative_fixture_citation_prefix_glob_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {"foo/a.md": "see ../fixtures/srt/libsrt-*.bin and ../fixtures/srt/libsrt-\n", "fixtures/srt/libsrt-x.bin": "x\n"},
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_relative_fixture_citation_wrong_name_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {"foo/a.md": "see ../fixtures/rtp/rtp_simple.bin\n", "fixtures/rtp/rtp-simple.bin": "x\n"},
    )
    assert "rtp_simple.bin" in _errors(monkeypatch, tmp_path, capsys)


def test_fixture_helper_argument_missing_fails(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    _scratch(
        tmp_path,
        {"foo/tests/it/a.rs": 'let p = test_fixtures::path("ts/missing.ts");\n', "fixtures/ts/here.ts": "x\n"},
    )
    assert "ts/missing.ts" in _errors(monkeypatch, tmp_path, capsys)


def test_fixture_helper_argument_existing_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {
            "foo/tests/it/a.rs": 'let p = test_fixtures::path("ts/here.ts");\n'
            'let q = test_fixtures::read(&"ts/here.ts");\n'
            'let r = test_fixtures::read_to_string("ts/{dynamic}.ts");\n',
            "fixtures/ts/here.ts": "x\n",
        },
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_fixture_helper_argument_private_tier_passes(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "fixtures" / "ts").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "fixtures" / "ts" / "big.ts").write_text("x\n")
    _scratch(tmp_path, {"foo/tests/it/a.rs": 'let p = test_fixtures::path("ts/big.ts");\n'})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_fixture_helper_argument_missing_private_tier_fails(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified").mkdir(parents=True, exist_ok=True)
    _scratch(tmp_path, {"foo/tests/it/a.rs": 'let p = test_fixtures::path("ts/big.ts");\n'})
    assert "ts/big.ts" in _errors(monkeypatch, tmp_path, capsys)


def test_sentence_final_period_after_fixture_doc_citation_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {"docs/good.md": "provenance in fixtures/hls/PROVENANCE.md. Next.\n", "fixtures/hls/PROVENANCE.md": "x\n"},
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_comment_naming_moved_test_file_fails(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(
        tmp_path,
        {"foo/src/a.rs": "// proven by foo/tests/x.rs\n", "foo/tests/it/x.rs": "fn t() {}\n"},
    )
    assert "path does not exist: foo/tests/x.rs" in _errors(monkeypatch, tmp_path, capsys)


def test_comment_naming_existing_test_file_passes(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(
        tmp_path,
        {"foo/src/a.rs": "// proven by foo/tests/it/x.rs\n", "foo/tests/it/x.rs": "fn t() {}\n"},
    )
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_bare_test_path_in_doc_that_only_exists_under_it_fails(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(
        tmp_path,
        {"docs/design.md": "see `tests/x.rs`\n", "foo/tests/it/x.rs": "fn t() {}\n"},
    )
    assert "path does not exist: tests/x.rs" in _errors(monkeypatch, tmp_path, capsys)


def test_bare_src_path_resolving_in_a_member_passes(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(tmp_path, {"docs/design.md": "see `src/lib.rs`\n", "foo/src/lib.rs": "\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_comment_naming_missing_docs_script_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(
        tmp_path,
        {"foo/tests/it/a.rs": "//! regenerate with docs/logs/analysis/oracle.py\n", "docs/analysis/oracle.py": "\n"},
    )
    assert "path does not exist: docs/logs/analysis/oracle.py" in _errors(monkeypatch, tmp_path, capsys)


def test_string_literal_path_in_rust_is_not_a_comment_citation(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(tmp_path, {"foo/src/a.rs": 'const P: &str = "foo/tests/nothing.rs";\n'})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_path_in_third_party_crate_is_not_checked(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(cc, "WORKSPACE_MEMBERS", {"foo"})
    _scratch(tmp_path, {"foo/src/a.rs": "// see backon/src/backoff/exponential.rs\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_bare_spec_file_name_matching_no_file_fails(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified" / "dvb").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "specs-verified" / "dvb" / "44-ecm-repetition-rate-descriptor.md").write_text("x\n")
    _scratch(tmp_path, {"foo/src/a.rs": '//! Per en_301_192.md "Table 44"\n'})
    assert "spec file name matches no file: en_301_192.md" in _errors(monkeypatch, tmp_path, capsys)


def test_bare_spec_file_name_naming_a_real_file_passes(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.delenv("SKIP_PRIVATE_FIXTURES", raising=False)
    (tmp_path / "private" / "specs-verified" / "dvb").mkdir(parents=True, exist_ok=True)
    (tmp_path / "private" / "specs-verified" / "dvb" / "en_301_192.md").write_text("x\n")
    _scratch(tmp_path, {"foo/src/a.rs": "//! Per en_301_192.md Table 44\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_bare_spec_file_name_skipped_without_private(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/src/a.rs": "//! Per en_301_192.md Table 44\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_isobmff_citation_without_2026_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/src/a.rs": "//! ISO/IEC 14496-12:2015 section 4.2\n"})
    assert "must name the 2026 edition" in _errors(monkeypatch, tmp_path, capsys)


def test_isobmff_bare_citation_fails(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/src/a.rs": "//! ISO/IEC 14496-12 section 4.2\n"})
    assert "must name the 2026 edition" in _errors(monkeypatch, tmp_path, capsys)


def test_isobmff_2026_citation_passes(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/src/a.rs": "//! ISO/IEC 14496-12:2026 section 4.2 and 14496-15:2017 and 14496-12-notes\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_isobmff_historical_changelog_is_exempt(tmp_path, monkeypatch, capsys) -> None:
    _scratch(tmp_path, {"foo/CHANGELOG.md": "ISO/IEC 14496-12:2015 section 4.2\n"})
    assert _errors(monkeypatch, tmp_path, capsys) == ""


def test_k_iso15_edition_required(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"a.md": "see ISO/IEC 14496-15 §5.3.3\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_k_iso15_wrong_edition_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"a.md": "see ISO/IEC 14496-15:2012 §5.3.3\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_k_iso15_edition_passes(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"a.md": "ISO/IEC 14496-15:2017 §5.3.3, 14496-15:2022 §11, x-14496-15.md\n"})
    assert _run(monkeypatch, tmp_path) == 0
