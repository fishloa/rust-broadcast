#!/usr/bin/env python3
"""Read-only pre-flight: is the `crates-io` publish gate real?

    tools/check-release-env.py [--repo OWNER/REPO]

Every release lane publishes from a job with `environment: crates-io`. That only
gates anything if the environment (a) exists, (b) carries a `required_reviewers`
protection rule with at least one reviewer and (c) the `CARGO_REGISTRY_TOKEN`
secret is set. Without (b) the job never reaches the `waiting` state:
`push-release-tags.sh` then sees every tag publish the moment its gates pass,
with nobody approving, and nothing in the wave looks wrong until it is done.

Also reports `prevent_self_review`: when true, the person who pushes a tag cannot
approve its own run (a second reviewer is needed); when false, the pusher can.

Uses `gh api` / `gh secret list` only (read-only). Exit 0 = gate real, secret
present; 1 = a problem (each printed); 2 = gh unusable.
"""

from __future__ import annotations

import json
import subprocess
import sys

ENV = "crates-io"
SECRET = "CARGO_REGISTRY_TOKEN"


def gh(*args: str) -> tuple[int, str]:
    try:
        r = subprocess.run(["gh", *args], capture_output=True, text=True)
    except FileNotFoundError:
        return 127, "gh not found"
    return r.returncode, r.stdout if r.returncode == 0 else r.stderr + r.stdout


def evaluate(env: dict | None, secrets: list[str]) -> tuple[list[str], list[str]]:
    """(problems, notes) from the environment JSON (None = absent) and secret names."""
    problems: list[str] = []
    notes: list[str] = []
    if env is None:
        problems.append(f"environment `{ENV}` does not exist: every publish job would run ungated")
    else:
        rules = env.get("protection_rules") or []
        reviewers = [r for r in rules if r.get("type") == "required_reviewers"]
        if not reviewers:
            problems.append(
                f"environment `{ENV}` has NO required_reviewers rule (protection_rules={[r.get('type') for r in rules]}): "
                "publish jobs will not wait for approval"
            )
        for r in reviewers:
            who = [(x.get("reviewer") or {}).get("login") or (x.get("reviewer") or {}).get("slug") or "?" for x in r.get("reviewers") or []]
            if not who:
                problems.append(f"environment `{ENV}`: the required_reviewers rule lists no reviewer")
            notes.append(f"reviewers: {', '.join(who) or 'none'}")
            notes.append(f"prevent_self_review: {r.get('prevent_self_review')}")
            if r.get("prevent_self_review") and len(who) < 2:
                problems.append("prevent_self_review is true with fewer than 2 reviewers: the tag pusher can never approve their own run")
        notes.append(f"can_admins_bypass: {env.get('can_admins_bypass')}; deployment_branch_policy: {env.get('deployment_branch_policy')}")
    if SECRET not in secrets:
        problems.append(f"repository secret {SECRET} is not set (secrets: {', '.join(secrets) or 'none'})")
    return problems, notes


def main(argv: list[str]) -> int:
    repo = "fishloa/rust-broadcast"
    if "--repo" in argv:
        repo = argv[argv.index("--repo") + 1]
    rc, out = gh("api", f"repos/{repo}/environments/{ENV}")
    env = None
    if rc == 0:
        env = json.loads(out)
    elif "404" not in out and "Not Found" not in out:
        print(f"gh api failed: {out.strip()}", file=sys.stderr)
        return 2
    rc2, out2 = gh("secret", "list", "--repo", repo)
    if rc2 != 0:
        print(f"gh secret list failed: {out2.strip()}", file=sys.stderr)
        return 2
    secrets = [l.split("\t")[0].split()[0] for l in out2.splitlines() if l.strip()]
    problems, notes = evaluate(env, secrets)
    for n in notes:
        print(f"INFO  {n}")
    for p in problems:
        print(f"FAIL  {p}")
    print("crates-io gate OK" if not problems else f"{len(problems)} problem(s): fix in repo Settings -> Environments before the first tag")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
