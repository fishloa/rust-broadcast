"""tools/check-release-env.py: the crates-io environment must exist with its token.

The owner chose (2026-11-06) to publish without a manual approval step, so a
missing `required_reviewers` rule is reported, not failed; a present one must not
deadlock."""

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


def test_no_protection_rules_passes_and_says_tags_publish_unapproved():
    problems, notes = cre.evaluate({"name": "crates-io", "protection_rules": []}, ["CARGO_REGISTRY_TOKEN"])
    assert problems == []
    assert any("without manual approval" in n for n in notes)


def test_missing_environment_and_missing_secret_are_failures():
    problems, _ = cre.evaluate(None, [])
    assert any("does not exist" in p for p in problems) and any("CARGO_REGISTRY_TOKEN" in p for p in problems)


def test_an_optional_reviewer_rule_passes_and_reports_self_review():
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
    assert r.returncode == 0 and "without manual approval" in r.stdout and "crates-io environment OK" in r.stdout
    rule = dict(REVIEW, prevent_self_review=True)
    (tmp_path / "env.json").write_text(json.dumps({"protection_rules": [rule]}))
    r = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)
    assert r.returncode == 1 and "can never approve their own run" in r.stdout
