"""tools/push-release-tags.sh against a throwaway git repo + a bare "remote".

No network: `curl` (the crates.io sparse index) and `gh` (workflow runs) are
stubs on PATH, and the bare remote's post-receive hook plays crates.io -- it
"publishes" a crate when its tag arrives, unless told not to. Each exit status
is asserted TOGETHER with what actually reached the remote, because the old
script's failure was an exit code that disagreed with the remote's tag list.
"""

import os
import pathlib
import shutil
import stat
import subprocess

import pytest

REPO = pathlib.Path(__file__).parents[2]
SCRIPT = REPO / "tools" / "push-release-tags.sh"
BASH = shutil.which("bash")

CURL = """#!/bin/sh
for a; do url="$a"; done
echo "$url" >> "$FAKE/curl.log"
if [ -f "$FAKE/curl-500" ]; then printf '\\n500'; exit 0; fi
f="$FAKE/index/$(basename "$url")"
if [ -f "$f" ]; then cat "$f"; printf '\\n200'; else printf '\\n404'; fi
"""

GH = """#!/bin/sh
while [ $# -gt 0 ]; do [ "$1" = "--branch" ] && b="$2"; shift; done
[ -f "$FAKE/gh/$b" ] && cat "$FAKE/gh/$b"
exit 0
"""

GIT_SPY = """#!/bin/sh
echo "$@" >> "$FAKE/git-called"
exec /usr/bin/git "$@"
"""

HOOK = """#!/bin/sh
while read old new ref; do
  tag="${ref#refs/tags/}"
  echo "$tag" >> "$FAKE/pushed.log"
  [ -f "$FAKE/nopublish" ] && continue
  crate="${tag%-v*}"; ver="${tag##*-v}"
  mkdir -p "$FAKE/index"
  printf '{"name":"%s","vers":"%s"}\\n' "$crate" "$ver" >> "$FAKE/index/$crate"
done
"""


def _exe(path, text):
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def git(cwd, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    return subprocess.run(["/usr/bin/git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True).stdout


@pytest.fixture()
def env(tmp_path):
    fake = tmp_path / "fake"
    (fake / "gh").mkdir(parents=True)
    bin_ = tmp_path / "bin"
    bin_.mkdir()
    _exe(bin_ / "curl", CURL)
    _exe(bin_ / "gh", GH)

    remote = tmp_path / "remote.git"
    git(tmp_path, "init", "-q", "--bare", str(remote))
    _exe(remote / "hooks" / "post-receive", HOOK)

    repo = tmp_path / "repo"
    (repo / "tools").mkdir(parents=True)
    shutil.copy(SCRIPT, repo / "tools" / "push-release-tags.sh")
    for crate in ("cratea", "crateb"):
        (repo / crate).mkdir()
        (repo / crate / "Cargo.toml").write_text(f'[package]\nname = "{crate}"\nversion = "1.0.0"\n')
    git(repo, "init", "-q")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "release")
    git(repo, "remote", "add", "origin", str(remote))
    for t in ("cratea-v1.0.0", "crateb-v1.0.0"):
        git(repo, "tag", t)

    e = dict(os.environ, FAKE=str(fake), PATH=f"{bin_}:{os.environ['PATH']}", POLL_SECS="0", MAX_POLLS="3", RUN_POLLS="3")

    class Ctx:
        pass

    c = Ctx()
    c.fake, c.repo, c.remote, c.env, c.bin = fake, repo, remote, e, bin_

    def run(*args, **extra):
        r = subprocess.run([BASH, "tools/push-release-tags.sh", *args], cwd=repo, env={**e, **extra}, capture_output=True, text=True, timeout=60)
        return r

    def remote_tags():
        return sorted(git(remote, "tag", "-l").split())

    c.run, c.remote_tags = run, remote_tags
    c.gh = lambda tag, text: (fake / "gh" / tag).write_text(text)
    return c


def test_happy_path_pushes_in_order_and_exits_zero(env):
    env.gh("cratea-v1.0.0", "completed success\n")
    env.gh("crateb-v1.0.0", "completed success\n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 0, r.stdout + r.stderr
    assert env.remote_tags() == ["cratea-v1.0.0", "crateb-v1.0.0"]
    # one at a time, in the order given
    assert (env.fake / "pushed.log").read_text().split() == ["cratea-v1.0.0", "crateb-v1.0.0"]


def test_failed_workflow_is_nonzero_and_stops_before_the_next_tag(env):
    env.gh("cratea-v1.0.0", "completed failure\n")
    env.gh("crateb-v1.0.0", "completed success\n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 1, r.stdout
    assert "FAILED" in r.stdout
    assert env.remote_tags() == ["cratea-v1.0.0"], "the next tag must NOT be pushed after a failure"


def test_published_but_workflow_never_finishes_is_nonzero(env):
    env.gh("cratea-v1.0.0", "in_progress \n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 1, r.stdout
    assert env.remote_tags() == ["cratea-v1.0.0"]


def test_approval_gate_wait_is_exit_2_and_stops(env):
    (env.fake / "nopublish").write_text("")
    env.gh("cratea-v1.0.0", "waiting \n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 2, r.stdout
    assert "approval gate" in r.stdout
    assert env.remote_tags() == ["cratea-v1.0.0"]


def test_index_never_shows_the_version_is_nonzero(env):
    (env.fake / "nopublish").write_text("")  # tag pushed, nothing publishes, no run exists: the #933 shape
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 1, r.stdout
    assert "#933" in r.stdout
    assert env.remote_tags() == ["cratea-v1.0.0"]


def test_lockstep_tag_refused_and_nothing_at_all_is_pushed(env):
    git(env.repo, "tag", "v1.0.0")
    r = env.run("cratea-v1.0.0", "v1.0.0")
    assert r.returncode == 1
    assert "REFUSED" in r.stdout and "lockstep" in r.stdout
    assert env.remote_tags() == [], "a refusal anywhere in the list must abort before the first push"


def test_missing_local_tag_and_manifest_mismatch_abort_before_any_push(env):
    r = env.run("cratea-v1.0.0", "crateb-v2.0.0")
    assert r.returncode == 1 and "says 1.0.0, tag says 2.0.0" in r.stdout
    git(env.repo, "tag", "-d", "crateb-v1.0.0")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 1 and "no LOCAL tag" in r.stdout
    assert env.remote_tags() == []


def test_unreadable_index_is_an_error_not_unpublished(env):
    (env.fake / "curl-500").write_text("")
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 1 and "could not read the crates.io index" in r.stdout
    assert env.remote_tags() == []


def test_already_live_is_skipped_without_pushing(env):
    (env.fake / "index").mkdir()
    (env.fake / "index" / "cratea").write_text('{"vers":"1.0.0"}\n')
    env.gh("crateb-v1.0.0", "completed success\n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 0, r.stdout
    assert "SKIP cratea-v1.0.0" in r.stdout
    assert env.remote_tags() == ["crateb-v1.0.0"]


def test_push_failure_is_nonzero(env):
    r = env.run("cratea-v1.0.0", REMOTE="/nonexistent/remote.git")
    assert r.returncode == 1 and "git push failed" in r.stdout


def test_dry_run_touches_neither_git_nor_network_nor_remote(env):
    _exe(env.bin / "git", GIT_SPY)
    r = env.run("--dry-run", "cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 0, r.stdout + r.stderr
    assert "1. cratea-v1.0.0" in r.stdout and "2. crateb-v1.0.0" in r.stdout
    assert not (env.fake / "git-called").exists(), "dry run ran git"
    assert not (env.fake / "curl.log").exists(), "dry run hit the network"
    assert env.remote_tags() == []


def test_dry_run_still_refuses_lockstep_arg(env):
    r = env.run("--dry-run", "v1.0.0")
    assert r.returncode == 1 and "REFUSED" in r.stdout


def test_plan_dry_run_prints_the_release_order_from_the_pair_list():
    """The real repo, the real plan: broadcast-common first, the lockstep slot
    after mpeg-ts and flagged MANUAL, every pair exactly once."""
    pairs = (REPO / "docs" / "release-plan-2026-10.pairs").read_text().split()
    r = subprocess.run([BASH, str(SCRIPT), "--dry-run", "--plan", str(REPO / "docs" / "release-plan-2026-10.pairs")], cwd=REPO, capture_output=True, text=True, timeout=120)
    assert r.returncode == 0, r.stdout + r.stderr
    tags = [l.split()[1] for l in r.stdout.splitlines() if l[:4].strip().rstrip(".").isdigit()]
    assert tags[0] == "broadcast-common-v10.0.0"
    assert len(tags) == len(set(tags)) == len(pairs) - 4  # the lockstep five share one slot
    assert tags.index("mp4-emsg-v0.5.0") < tags.index("mpeg-ts-v0.6.0") < tags.index("v12.0.0") < tags.index("transmux-v0.27.0")
    assert tags[-1] == "multimux-cli-v0.11.0"
    assert "v12.0.0  MANUAL" in r.stdout
