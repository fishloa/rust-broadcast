"""tools/check-release-env.py: the crates-io approval gate must be REAL.

Observed live on 2026-10-17: the environment existed with `protection_rules: []`, i.e.
no required reviewer, so no release job would ever have waited for approval."""

import importlib.util
import json
import pathlib
import stat
import subprocess
import sys

SCRIPT = pathlib.Path(__file__).parents[1] / "check-release-env.py"
spec = importlib.util.spec_from_file_location("check_release_env", SCRIPT)
cre = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cre)

REVIEW = {"type": "required_reviewers", "prevent_self_review": False, "reviewers": [{"type": "User", "reviewer": {"login": "owner"}}]}


def test_no_protection_rules_is_a_failure_naming_the_cause():
    problems, _ = cre.evaluate({"name": "crates-io", "protection_rules": []}, ["CARGO_REGISTRY_TOKEN"])
    assert any("NO required_reviewers rule" in p for p in problems)


def test_missing_environment_and_missing_secret_are_failures():
    problems, _ = cre.evaluate(None, [])
    assert any("does not exist" in p for p in problems) and any("CARGO_REGISTRY_TOKEN" in p for p in problems)


def test_a_real_gate_passes_and_reports_self_review():
    problems, notes = cre.evaluate({"protection_rules": [REVIEW], "can_admins_bypass": True}, ["CARGO_REGISTRY_TOKEN", "X"])
    assert problems == []
    assert "reviewers: owner" in notes and "prevent_self_review: False" in notes


def test_prevent_self_review_with_a_single_reviewer_is_a_deadlock():
    rule = dict(REVIEW, prevent_self_review=True)
    problems, _ = cre.evaluate({"protection_rules": [rule]}, ["CARGO_REGISTRY_TOKEN"])
    assert any("can never approve their own run" in p for p in problems)
    rule["reviewers"] = rule["reviewers"] * 2
    assert cre.evaluate({"protection_rules": [rule]}, ["CARGO_REGISTRY_TOKEN"])[0] == []


def test_cli_with_a_stub_gh(tmp_path):
    gh = tmp_path / "gh"
    gh.write_text(
        "#!/bin/sh\n"
        'case "$1" in\n'
        f"  api) cat {tmp_path}/env.json ;;\n"
        '  secret) printf "CARGO_REGISTRY_TOKEN\\t2026-06-05T22:17:30Z\\n" ;;\n'
        "esac\n"
    )
    gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
    env = {"PATH": f"{tmp_path}:/usr/bin:/bin"}
    (tmp_path / "env.json").write_text(json.dumps({"protection_rules": []}))
    r = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)
    assert r.returncode == 1 and "NO required_reviewers" in r.stdout
    (tmp_path / "env.json").write_text(json.dumps({"protection_rules": [REVIEW]}))
    r = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)
    assert r.returncode == 0 and "crates-io gate OK" in r.stdout
