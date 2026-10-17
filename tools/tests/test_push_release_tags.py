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
# PUSH_RELEASE_TAGS_SCRIPT lets a reviewer point the suite at an OLD copy of the script to
# watch the new tests fail (the "tests must bite" check).
SCRIPT = pathlib.Path(os.environ.get("PUSH_RELEASE_TAGS_SCRIPT") or REPO / "tools" / "push-release-tags.sh")
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
    shutil.copy(REPO / "tools" / "release_deps.py", repo / "tools" / "release_deps.py")
    notes = repo / "docs" / "release-notes"
    notes.mkdir(parents=True)
    for crate in ("cratea", "crateb", "cratec"):
        (repo / crate).mkdir()
        (repo / crate / "Cargo.toml").write_text(f'[package]\nname = "{crate}"\nversion = "1.0.0"\n')
        (notes / f"{crate}-1.0.0.md").write_text(f"# {crate} 1.0.0\n\nbody\n")
    # lockstep five at 12.0.0; dvb-tools has one EXTERNAL workspace dep (extdep "1.0")
    # and one lane-internal dep (dvb-si), which must not be demanded from the index.
    for crate in ("dvb-bbframe", "dvb-si", "dvb-conformance", "dvb-t2mi", "dvb-tools"):
        (repo / crate).mkdir()
        extra = (
            '\n[dependencies]\nextdep = { path = "../extdep", version = "1.0" }\n'
            'dvb-si = { path = "../dvb-si", version = "12.0" }\n'
            if crate == "dvb-tools"
            else ""
        )
        (repo / crate / "Cargo.toml").write_text(f'[package]\nname = "{crate}"\nversion = "12.0.0"\n{extra}')
    git(repo, "init", "-q")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "release")
    git(repo, "remote", "add", "origin", str(remote))
    for t in ("cratea-v1.0.0", "crateb-v1.0.0", "cratec-v1.0.0", "v12.0.0"):
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
    c.git = lambda *a: git(repo, *a)

    def commit_file(rel, text):
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        (repo / rel).write_text(text)
        git(repo, "add", ".")
        git(repo, "commit", "-q", "-m", f"edit {rel}")

    c.commit_file = commit_file

    def index(crate, *versions, yanked=()):
        (fake / "index").mkdir(exist_ok=True)
        lines = "".join('{"name":"%s","vers":"%s","yanked":%s}\n' % (crate, v, "true" if v in yanked else "false") for v in versions)
        (fake / "index" / crate).write_text(lines)

    c.index = index
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
    git(env.repo, "tag", "crateb-v2.0.0")
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
    assert r.returncode == 1 and "could not read the tags of remote" in r.stdout
    assert env.remote_tags() == []


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


# ---- round 3: dependency liveness, tagged-tree preflight, remote pre-check ----------

DEP_B_ON_A = '[package]\nname = "crateb"\nversion = "1.0.0"\n\n[dependencies]\ncratea = { path = "../cratea", version = "1.0" }\n'


def _retag_b_with_dep(env):
    env.commit_file("crateb/Cargo.toml", DEP_B_ON_A)
    env.git("tag", "-f", "crateb-v1.0.0")
    env.git("tag", "-f", "cratea-v1.0.0")
    env.git("tag", "-f", "cratec-v1.0.0")
    env.git("tag", "-f", "v12.0.0")


def test_explicit_tag_refuses_when_a_dependency_is_not_live(env):
    _retag_b_with_dep(env)
    r = env.run("crateb-v1.0.0")
    assert r.returncode == 2, r.stdout
    assert "cratea" in r.stdout and "not on crates.io" in r.stdout
    assert env.remote_tags() == [], "nothing may be pushed when a dependency is missing"


def test_dependency_live_but_below_the_required_version_is_refused(env):
    _retag_b_with_dep(env)
    env.index("cratea", "0.9.0")
    r = env.run("crateb-v1.0.0")
    assert r.returncode == 2 and "cratea 1.0" in r.stdout and "no live version satisfies" in r.stdout
    assert env.remote_tags() == []


def test_yanked_dependency_version_does_not_satisfy(env):
    _retag_b_with_dep(env)
    env.index("cratea", "1.0.0", yanked=("1.0.0",))
    r = env.run("crateb-v1.0.0")
    assert r.returncode == 2 and env.remote_tags() == []


def test_dependency_satisfied_by_max_semver_not_last_index_line(env):
    """Publish order puts a patch of an OLD line last; that must not hide 1.2.0."""
    _retag_b_with_dep(env)
    env.index("cratea", "0.9.0", "1.2.0", "0.9.1")
    env.gh("crateb-v1.0.0", "completed success\n")
    r = env.run("crateb-v1.0.0")
    assert r.returncode == 0, r.stdout
    assert env.remote_tags() == ["crateb-v1.0.0"]


def test_plan_mode_pushes_dependency_then_dependent(env):
    """In plan order the dependency is pushed (and goes live) before its dependent's check."""
    _retag_b_with_dep(env)
    env.gh("cratea-v1.0.0", "completed success\n")
    env.gh("crateb-v1.0.0", "completed success\n")
    r = env.run("cratea-v1.0.0", "crateb-v1.0.0")
    assert r.returncode == 0, r.stdout
    assert (env.fake / "pushed.log").read_text().split() == ["cratea-v1.0.0", "crateb-v1.0.0"]


def _stub_plan(env, tags):
    stub = env.repo / "tools" / "order.py"
    stub.write_text("print(%r)\n" % "\n".join(tags))
    pairs = env.repo / "pairs"
    pairs.write_text("cratea=1.0.0\n")
    return {"PREPARE_RELEASE": str(stub)}, str(pairs)


def test_lockstep_slot_checks_all_five_crates_external_deps(env):
    extra, pairs = _stub_plan(env, ["cratea-v1.0.0", "v12.0.0"])
    env.gh("cratea-v1.0.0", "completed success\n")
    r = env.run("--plan", pairs, **extra)
    assert r.returncode == 2, r.stdout
    assert "dvb-tools depends on extdep" in r.stdout and "Do NOT push v12.0.0" in r.stdout
    # the lane-internal edge dvb-tools -> dvb-si is NOT demanded from the index
    assert "dvb-si 12.0" not in r.stdout
    env.index("extdep", "1.0.3")
    r = env.run("--plan", pairs, **extra)
    assert r.returncode == 2 and "Push it BY HAND" in r.stdout


def test_local_tag_must_point_at_head(env):
    env.commit_file("docs/unrelated.txt", "x\n")  # HEAD moves past the tags
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 1 and "HEAD is" in r.stdout and "--allow-tag-commit" in r.stdout
    assert env.remote_tags() == []
    pinned = env.git("rev-parse", "cratea-v1.0.0^{commit}").strip()
    env.gh("cratea-v1.0.0", "completed success\n")
    r = env.run("--allow-tag-commit", pinned, "cratea-v1.0.0")
    assert r.returncode == 0, r.stdout
    assert env.remote_tags() == ["cratea-v1.0.0"]


def test_preflight_reads_the_tagged_manifest_not_the_working_tree(env):
    """The working tree says 2.0.0 (uncommitted) but the TAGGED manifest says 1.0.0."""
    (env.repo / "cratea" / "Cargo.toml").write_text('[package]\nname = "cratea"\nversion = "2.0.0"\n')
    env.git("tag", "cratea-v2.0.0")
    r = env.run("cratea-v2.0.0")
    assert r.returncode == 1 and "at the tag says 1.0.0, tag says 2.0.0" in r.stdout
    assert env.remote_tags() == []


def test_preflight_reads_the_tagged_release_note(env):
    # tag cratea-v1.0.0 sits on a commit whose note is DRAFT; the working tree (HEAD) has it fixed
    env.commit_file("docs/release-notes/cratea-1.0.0.md", "---\nstatus: DRAFT\n---\n\n# t\n\nbody\n")
    env.git("tag", "-f", "cratea-v1.0.0")
    env.commit_file("docs/release-notes/cratea-1.0.0.md", "# fixed\n\nbody\n")
    pinned = env.git("rev-parse", "cratea-v1.0.0^{commit}").strip()
    r = env.run("--allow-tag-commit", pinned, "cratea-v1.0.0")
    assert r.returncode == 1 and "missing/empty/DRAFT" in r.stdout and "DRAFT" in r.stdout
    assert env.remote_tags() == []


def test_missing_release_note_at_the_tag_is_refused(env):
    env.git("rm", "-q", "docs/release-notes/cratea-1.0.0.md")
    env.git("commit", "-q", "-m", "drop note")
    env.git("tag", "-f", "cratea-v1.0.0")
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 1 and "release note docs/release-notes/cratea-1.0.0.md" in r.stdout
    assert env.remote_tags() == []


def test_stale_remote_tag_at_another_commit_is_refused(env):
    (env.fake / "nopublish").write_text("")
    env.git("push", "-q", str(env.remote), "cratea-v1.0.0")  # remote now has the tag at the old commit
    env.commit_file("docs/unrelated.txt", "x\n")
    env.git("tag", "-f", "cratea-v1.0.0")  # re-tagged locally at the new HEAD
    (env.fake / "pushed.log").unlink(missing_ok=True)
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 1 and "already has cratea-v1.0.0" in r.stdout and "#933" in r.stdout
    assert not (env.fake / "pushed.log").exists(), "nothing may be pushed"


def test_remote_tag_without_any_release_run_is_the_933_shape(env):
    (env.fake / "nopublish").write_text("")
    env.git("push", "-q", str(env.remote), "cratea-v1.0.0")
    (env.fake / "pushed.log").unlink(missing_ok=True)
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 1 and "NO release workflow run exists" in r.stdout


def test_remote_tag_with_a_release_run_resumes_without_pushing(env):
    (env.fake / "nopublish").write_text("")
    env.git("push", "-q", str(env.remote), "cratea-v1.0.0")
    (env.fake / "pushed.log").unlink(missing_ok=True)
    env.index("cratea", "1.0.0")  # the run has since published
    # live already => SKIP; and for a not-yet-live one the run state decides
    r = env.run("cratea-v1.0.0")
    assert r.returncode == 0 and "SKIP cratea-v1.0.0" in r.stdout
    (env.fake / "nopublish").write_text("")
    env.git("push", "-q", str(env.remote), "crateb-v1.0.0")
    env.gh("crateb-v1.0.0", "waiting \n")
    r = env.run("crateb-v1.0.0")
    assert r.returncode == 2 and "resuming without a push" in r.stdout and "approval gate" in r.stdout
    assert not (env.fake / "pushed.log").exists()
