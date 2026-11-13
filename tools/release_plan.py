#!/usr/bin/env python3
"""Work out what to release, in which dependency levels, and push it in parallel (#2630).

Usage:
    tools/release_plan.py plan     [--no-live]
    tools/release_plan.py run      [--dry-run] [--cap N] [--allow-tag-commit SHA]...

`plan` reads only (cargo metadata, git tags/diffs, crates.io lookups) and prints:
  * to-publish       manifest version is not live on crates.io yet
  * needs-attention  source changed since the last release tag but the version
                     was not bumped / CHANGELOG [Unreleased] is empty, or the
                     manifest is BEHIND crates.io
  * live             already published at the manifest version (skipped)
  * the dependency levels of the to-publish set, and an ETA.

`run` pushes level by level. Within a level every crate gets its OWN
`tools/push-release-tags.sh <one-tag>` process (one single-tag `git push` each,
never more than one tag per push: GitHub starts no workflows when more than 3
tags arrive in one push, #933), up to --cap at once. The level is finished when
every crate is on the crates.io index at the new version AND its release
workflow succeeded; only then does the next level start. `--dry-run` runs the
same loop but every worker is `push-release-tags.sh --dry-run` (no git command,
no network, no push).

Why the push/wait logic is NOT reimplemented here: `push-release-tags.sh <tag>`
already carries every preflight (tagged tree has the version, release note not
DRAFT, local tag at HEAD or an allowed commit, remote tag at another commit
refused, workspace deps live at the required version) plus the index and
workflow polling. This tool adds only what the script cannot know: which tags,
in what order, how many at once. One implementation, one place to fix.

Exit status: 0 everything pushed/live; 1 a crate failed (nothing of a later
level was started); 2 stopped for a manual step (the lockstep `v<ver>` tag, a
release run waiting on the crates-io approval gate, a dependency not live yet);
64 usage; 3 plan found needs-attention items (plan mode only, informational).
A crates.io 429 is reported by name and never auto-retried.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import math
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS = Path(__file__).resolve().parent
LOCKSTEP = ("dvb-si", "dvb-t2mi", "dvb-bbframe", "dvb-conformance", "dvb-tools")
LOCKSTEP_NODE = "LOCKSTEP"
UNPUBLISHED_CRATES = {"atsc3"}
MINUTES_PER_CRATE = 1.7
DEFAULT_CAP = 6
DEFAULT_STAGGER_SECS = 3.0
SEMVER_TAG = re.compile(r"^(?P<crate>.+)-v(?P<ver>\d+\.\d+\.\d+)$")
LOCKSTEP_TAG = re.compile(r"^v(?P<ver>\d+\.\d+\.\d+)$")

EXIT_OK, EXIT_FAIL, EXIT_MANUAL, EXIT_ATTENTION, EXIT_USAGE = 0, 1, 2, 3, 64


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_MODS: dict = {}


def _cached(name: str, file: str):
    if name not in _MODS:
        _MODS[name] = _load(name, file)
    return _MODS[name]


def crates_io():
    return _cached("crates_io", "crates-io.py")


def release_deps():
    return _cached("release_deps", "release_deps.py")


@dataclass
class Crate:
    name: str
    version: str
    dir: str
    deps: set[str] = field(default_factory=set)


@dataclass
class Verdict:
    node: str
    tag: str
    version: str
    status: str  # to-publish | needs-attention | live
    reason: str = ""
    crates: tuple[str, ...] = ()


def sem_key(v: str) -> tuple:
    return release_deps().parse_version(v)


def node_of(crate: str) -> str:
    return LOCKSTEP_NODE if crate in LOCKSTEP else crate


def publish_edges(dependencies: list[dict], members: set[str]) -> set[str]:
    """Workspace edges the publish order honours: normal + build deps, and dev
    deps only when versioned (`req != "*"`; a path-only dev-dep is stripped at
    publish and gates nothing). Same rule as prepare-release.py `_publish_deps`."""
    out: set[str] = set()
    for d in dependencies:
        if d["name"] not in members:
            continue
        if d.get("kind") == "dev" and d.get("req", "*") == "*":
            continue
        out.add(d["name"])
    return out


def load_workspace(metadata: dict) -> dict[str, Crate]:
    pkgs = metadata["packages"]
    members = {p["name"] for p in pkgs}
    out: dict[str, Crate] = {}
    for p in pkgs:
        if p.get("publish") == [] or p["name"] in UNPUBLISHED_CRATES:
            continue
        out[p["name"]] = Crate(
            name=p["name"],
            version=p["version"],
            dir=Path(p["manifest_path"]).parent.name,
            deps=publish_edges(p["dependencies"], members) - {p["name"]},
        )
    return out


def compute_levels(deps: dict[str, set[str]]) -> list[list[str]]:
    """Topological layers. `deps` maps node -> nodes it needs live first; edges
    to nodes outside `deps` are already satisfied and ignored."""
    remaining = {n: {d for d in ds if d in deps and d != n} for n, ds in deps.items()}
    levels: list[list[str]] = []
    done: set[str] = set()
    while remaining:
        ready = sorted(n for n, ds in remaining.items() if ds <= done)
        if not ready:
            raise ValueError(f"dependency cycle among: {sorted(remaining)}")
        levels.append(ready)
        done.update(ready)
        for n in ready:
            del remaining[n]
    return levels


def node_deps(verdicts: list[Verdict], workspace: dict[str, Crate]) -> dict[str, set[str]]:
    in_set = {v.node for v in verdicts}
    out: dict[str, set[str]] = {}
    for v in verdicts:
        needs: set[str] = set()
        for c in v.crates:
            for d in workspace[c].deps:
                n = node_of(d)
                if n in in_set and n != v.node:
                    needs.add(n)
        out[v.node] = needs
    return out


def unreleased_body(changelog: str) -> str:
    m = re.search(r"^## \[Unreleased\][^\n]*\n(.*?)(?=^## \[|\Z)", changelog, re.M | re.S)
    return m.group(1).strip() if m else ""


def classify(
    node: str,
    crates: list[Crate],
    live: dict[str, str | None],
    last_tag_version: str | None,
    changed_since_tag: bool,
    unreleased_empty: bool,
) -> Verdict:
    """The decision table. `live[c]` is None for a crate not on crates.io."""
    names = tuple(sorted(c.name for c in crates))
    versions = {c.version for c in crates}
    tag_fn = (lambda v: f"v{v}") if node == LOCKSTEP_NODE else (lambda v: f"{node}-v{v}")
    if len(versions) != 1:
        return Verdict(node, "", "", "needs-attention", f"lockstep crates disagree on the version: {sorted(versions)}", names)
    version = versions.pop()
    tag = tag_fn(version)
    lives = [live.get(c.name) for c in crates]
    if all(v == version for v in lives):
        if last_tag_version == version and changed_since_tag:
            why = "CHANGELOG [Unreleased] is empty" if unreleased_empty else "CHANGELOG [Unreleased] has entries"
            return Verdict(node, tag, version, "needs-attention", f"source changed since {tag_fn(version)} but the version is not bumped ({why})", names)
        return Verdict(node, tag, version, "live", "", names)
    known = [v for v in lives if v is not None]
    if known and any(sem_key(v) > sem_key(version) for v in known):
        return Verdict(node, tag, version, "needs-attention", f"manifest {version} is BEHIND crates.io ({max(known, key=sem_key)})", names)
    if last_tag_version == version and changed_since_tag and unreleased_empty:
        return Verdict(node, tag, version, "needs-attention", f"source changed since {tag} and CHANGELOG [Unreleased] is empty", names)
    return Verdict(node, tag, version, "to-publish", "", names)


# ---- git / network (all read-only) -------------------------------------------------


def git(*args: str) -> str:
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def last_tags() -> tuple[dict[str, tuple[str, str]], tuple[str, str] | None]:
    """crate -> (tag, version) newest by semver; and the newest lockstep tag."""
    best: dict[str, tuple[str, str]] = {}
    lock: tuple[str, str] | None = None
    for t in git("tag", "--list").split():
        m = SEMVER_TAG.match(t)
        if m:
            c, v = m["crate"], m["ver"]
            if c not in best or sem_key(v) > sem_key(best[c][1]):
                best[c] = (t, v)
            continue
        m = LOCKSTEP_TAG.match(t)
        if m and (lock is None or sem_key(m["ver"]) > sem_key(lock[1])):
            lock = (t, m["ver"])
    return best, lock


def source_changed(tag: str, dirs: list[str]) -> bool:
    out = git("diff", "--name-only", f"{tag}..HEAD", "--", *dirs)
    return any(Path(f).name != "CHANGELOG.md" for f in out.split())


def live_versions(names: list[str], lookup=None, sleep=time.sleep, tries: int = 4) -> dict[str, str | None]:
    """crates.io lookups through tools/crates-io.py (User-Agent, 404 == absent,
    anything else is an error). A 429 is retried with backoff, then raised."""
    cio = crates_io()
    lookup = lookup or cio.live_version
    out: dict[str, str | None] = {}

    def one(name: str):
        for attempt in range(tries):
            try:
                return lookup(name)
            except cio.LookupError_ as e:
                if "429" in str(e) and attempt + 1 < tries:
                    sleep(2 ** (attempt + 1))
                    continue
                raise

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for name, v in zip(names, pool.map(one, names)):
            out[name] = v
    return out


def build_verdicts(workspace: dict[str, Crate], live: dict[str, str | None]) -> list[Verdict]:
    tags, lock_tag = last_tags()
    groups: dict[str, list[Crate]] = {}
    for c in workspace.values():
        groups.setdefault(node_of(c.name), []).append(c)
    verdicts = []
    for node, crates in sorted(groups.items()):
        if node == LOCKSTEP_NODE:
            ltag = lock_tag
        else:
            ltag = tags.get(node)
        changed, empty = False, True
        if ltag:
            changed = source_changed(ltag[0], [c.dir for c in crates])
            texts = [(ROOT / c.dir / "CHANGELOG.md") for c in crates]
            empty = all(not unreleased_body(p.read_text()) for p in texts if p.exists())
        verdicts.append(classify(node, crates, live, ltag[1] if ltag else None, changed, empty))
    return verdicts


def eta_minutes(levels: list[list[str]], cap: int) -> tuple[float, float]:
    n = sum(len(level) for level in levels)
    parallel = sum(MINUTES_PER_CRATE * math.ceil(len(level) / cap) for level in levels)
    return parallel, MINUTES_PER_CRATE * n


def render_plan(verdicts: list[Verdict], levels: list[list[str]], cap: int) -> str:
    by = {v.node: v for v in verdicts}
    out: list[str] = []
    for status in ("to-publish", "needs-attention", "live"):
        group = [v for v in verdicts if v.status == status]
        out.append(f"{status} ({len(group)})")
        for v in group:
            if status == "live":
                continue
            out.append(f"  {v.tag or v.node}" + (f"  -- {v.reason}" if v.reason else ""))
    out.append("")
    out.append(f"levels ({len(levels)}), cap {cap} concurrent single-tag pushes:")
    for i, level in enumerate(levels):
        tags = [by[n].tag + (" [MANUAL]" if n == LOCKSTEP_NODE else "") for n in level]
        out.append(f"  L{i} ({len(level)}): " + " ".join(tags))
    par, seq = eta_minutes(levels, cap)
    out.append(f"ETA ~{par:.0f} min parallel vs ~{seq:.0f} min one-at-a-time (at {MINUTES_PER_CRATE} min/crate; the manual lockstep step is extra)")
    return "\n".join(out)


# ---- the run -----------------------------------------------------------------------

Runner = "callable(tag: str) -> tuple[int, str]"


def script_runner(dry: bool, allow: list[str], script: Path | None = None, log_dir: Path | None = None):
    script = script or TOOLS / "push-release-tags.sh"

    def run(tag: str) -> tuple[int, str]:
        argv = ["bash", str(script)]
        if dry:
            argv.append("--dry-run")
        for sha in allow:
            argv += ["--allow-tag-commit", sha]
        argv.append(tag)
        r = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True)
        out = r.stdout + r.stderr
        if log_dir:
            log_dir.mkdir(parents=True, exist_ok=True)
            (log_dir / f"{tag}.log").write_text(out)
        return r.returncode, out

    return run


def rate_limited(tag: str, gh=None) -> bool:
    """True when the failed release run of `tag` shows a crates.io 429."""
    gh = gh or (lambda *a: subprocess.run(["gh", *a], cwd=ROOT, capture_output=True, text=True).stdout)
    try:
        runs = json.loads(gh("run", "list", "--branch", tag, "--limit", "5", "--json", "databaseId,conclusion") or "[]")
        for r in runs:
            if r.get("conclusion") == "failure":
                log = gh("run", "view", str(r["databaseId"]), "--log-failed")
                if re.search(r"\b429\b|Too Many Requests", log):
                    return True
    except (ValueError, KeyError, OSError):
        return False
    return False


def lockstep_live(version: str, live=None) -> bool:
    live = live or crates_io().live_version
    return all(live(c) == version for c in LOCKSTEP)


def run_levels(
    verdicts: list[Verdict],
    levels: list[list[str]],
    runner,
    cap: int = DEFAULT_CAP,
    stagger: float = DEFAULT_STAGGER_SECS,
    sleep=time.sleep,
    lockstep_is_live=lockstep_live,
    detect_429=rate_limited,
    out=print,
) -> int:
    by = {v.node: v for v in verdicts}
    for i, level in enumerate(levels):
        pushable = [n for n in level if n != LOCKSTEP_NODE]
        out(f"=== level {i}: {' '.join(by[n].tag for n in level)}")

        def start(item):
            idx, n = item
            sleep(stagger * (idx % cap))
            return n, runner(by[n].tag)

        results: list[tuple[str, tuple[int, str]]] = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, cap)) as pool:
            for r in pool.map(start, list(enumerate(pushable))):
                results.append(r)
        worst = EXIT_OK
        for n, (rc, text) in results:
            tag = by[n].tag
            if rc == 0:
                out(f"  OK   {tag}")
                continue
            tail = " | ".join(text.strip().splitlines()[-2:])
            if rc == 2:
                out(f"  STOP {tag} (manual step): {tail}")
                worst = max(worst, EXIT_MANUAL) if worst != EXIT_FAIL else worst
            else:
                note = ""
                if detect_429(tag):
                    note = "  [crates.io 429 rate limit: wait, then re-run the failed workflow by hand; not auto-retried]"
                out(f"  FAIL({rc}) {tag}: {tail}{note}")
                worst = EXIT_FAIL
        if worst != EXIT_OK:
            out(f"STOP at level {i}; later levels were NOT started.")
            return worst
        if LOCKSTEP_NODE in level:
            v = by[LOCKSTEP_NODE]
            if not lockstep_is_live(v.version):
                out(f"STOP: level {i} contains the lockstep tag {v.tag}. Push it BY HAND, approve its crates-io gate,")
                out("      wait until the five crates are live, then re-run this command.")
                return EXIT_MANUAL
            out(f"  OK   {v.tag} (lockstep live)")
    out("ALL LEVELS DONE")
    return EXIT_OK


def missing_local_tags(verdicts: list[Verdict]) -> list[str]:
    have = set(git("tag", "--list").split())
    return [v.tag for v in verdicts if v.tag not in have]


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("plan", "run"):
        p = sub.add_parser(name)
        p.add_argument("--cap", type=int, default=DEFAULT_CAP)
        if name == "run":
            p.add_argument("--dry-run", action="store_true")
            p.add_argument("--allow-tag-commit", action="append", default=[])
            p.add_argument("--log-dir", type=Path, default=ROOT / ".delegate" / "scratch" / "release-logs")
            p.add_argument("--stagger", type=float, default=DEFAULT_STAGGER_SECS)
    args = ap.parse_args(argv)
    if args.cap < 1:
        print("--cap must be >= 1")
        return EXIT_USAGE

    meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"], cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    workspace = load_workspace(meta)
    try:
        live = live_versions(sorted(workspace))
    except crates_io().LookupError_ as e:
        print(f"FAIL: could not read crates.io ({e}); not guessing which crates are live.")
        return EXIT_FAIL
    verdicts = build_verdicts(workspace, live)
    to_publish = [v for v in verdicts if v.status == "to-publish"]
    levels = compute_levels(node_deps(to_publish, workspace))
    print(render_plan(verdicts, levels, args.cap))
    attention = [v for v in verdicts if v.status == "needs-attention"]

    if args.cmd == "plan":
        return EXIT_ATTENTION if attention else EXIT_OK
    if attention:
        print("REFUSED: needs-attention items must be resolved first; nothing was pushed.")
        return EXIT_FAIL
    if not args.dry_run:
        missing = missing_local_tags(to_publish)
        if missing:
            print(f"REFUSED: local tags missing (create them at the release commit first): {' '.join(missing)}")
            return EXIT_FAIL
    runner = script_runner(args.dry_run, args.allow_tag_commit, log_dir=args.log_dir)
    return run_levels(to_publish, levels, runner, cap=args.cap, stagger=0 if args.dry_run else args.stagger)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
