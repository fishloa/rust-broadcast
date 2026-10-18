# Release runbook - the 2026-10 breaking wave

The mechanical, ordered command list for the release described in
[`release-plan-2026-10.md`](release-plan-2026-10.md) (sections 8-10). The pair list is
[`release-plan-2026-10.pairs`](release-plan-2026-10.pairs): 54 crates, 50 tags (49
independent + the lockstep `v12.0.0`). Audit evidence and the must-fix list:
[`release-audit-2026-10.md`](release-audit-2026-10.md).

Nothing here runs until the owner says "release". Never `cargo publish` from a
workstation; CI is the only publisher. Every `cargo` and gate command is run on its own
line and its exit status read (`echo "exit=$?"`), never chained with `&&` or looped:
a wrapper that returns 101 on success once turned an `&&` chain into a silent green.

```bash
PAIRS=docs/release-plan-2026-10.pairs
```

## 0. Before you start

* `main` is green on CI and the work tree is clean. Branch from `main`:
  `git switch -c release/2026-10 main`.
* The README coverage gaps, the CLAUDE.md "Unpublished" markers and the other audit
  must-fix items that need a human (`release-audit-2026-10.md`, "Must fix before tag" 3-8)
  are landed or consciously waived. Item 1 (DRAFT notes) and 2 (`Cargo.lock`) are done by
  step 1 below. Write the final text of `docs/release-notes/v12.0.0.md` and review the other
  notes now: step 1 refuses a note that still contains a placeholder line.
* `gh auth status` is logged in as an account that can approve the `crates-io`
  environment (step 5a).
* **The publish gate must be real.** Read-only pre-flight (`gh api` + `gh secret list`):

  ```bash
  python3 tools/check-release-env.py
  ```

  It fails unless the `crates-io` environment exists with a `required_reviewers` rule
  that names at least one reviewer, and `CARGO_REGISTRY_TOKEN` is a set repository secret.
  It also prints `prevent_self_review`: when `true` the account that pushes a tag
  **cannot approve its own run**, so a second reviewer is required (the check flags a
  single-reviewer deadlock). **Finding on 2026-10-17: the environment exists with
  `protection_rules: []` (no reviewer at all) and `can_admins_bypass: true`.** As it stands
  no run would ever reach `waiting`: every tag would publish the moment its lane gates pass,
  `push-release-tags.sh`'s exit-2 "waiting" branch could never fire, and section 5a would
  have nothing to approve. Add the required reviewer(s) in repo Settings -> Environments ->
  `crates-io` and re-run this check until it prints `crates-io gate OK`. The secret was
  present (`CARGO_REGISTRY_TOKEN`, set 2026-06-05).
* **The identical gate on CI, BEFORE the first tag.** The lockstep lane (`release.yml`)
  runs its test gate only after ~33 crates are live, so a failure there strands the wave
  halfway; it is a superset of CI's (CI excludes `dvb-tools` and `dvb-conformance`, and runs
  nextest, which `release.yml` does not; both lanes check out the private fixtures, below). So, on the
  release commit (the merge commit of step 3): every check of the CI run for that commit is
  SUCCESS (`gh run list --commit <sha>`), and the two crates CI excludes pass on their own:

  ```bash
  cargo test -p dvb-tools -p dvb-conformance --all-features --locked
  ```

  (`release.yml`'s test gate now also carries `timeout-minutes: 60`.)

## 1. Prepare: one command

Preview first. It prints every manifest, sibling requirement, README snippet, CHANGELOG
cut and note it would touch, the notes it would finalise, the crate-index and lock steps,
and the tag order. It writes nothing; exit 0 means no note blocks the apply.

```bash
python3 tools/prepare-release.py --check $(cat $PAIRS) --extra-note docs/release-notes/2026-10-wave-overview.md
```

Apply:

```bash
python3 tools/prepare-release.py $(cat $PAIRS) --extra-note docs/release-notes/2026-10-wave-overview.md
```

What apply does, in order (each step is also in the preview):

1. bumps `version` in each `Cargo.toml`;
2. points every sibling requirement (every manifest, incl. `fuzz/` and `demo/`, every
   dependency table) at the new `major.minor`;
3. rewrites README dependency snippets for every bumped crate (`tools/readme_snippets.py`
   logic: keeps the number of components the author wrote, never shortens a 0.x below
   `major.minor`);
4. cuts each CHANGELOG `[Unreleased]` to `## [X.Y.Z] - <today>` (a default "Dependency/epoch
   bump only" line for crates with nothing in it);
5. scaffolds a release note only where none exists (all 55 drafts exist);
6. **finalises the notes**: strips the `status: DRAFT` front-matter block from the 54 pair
   notes, `v12.0.0.md` and the `--extra-note` overview, and lists them. It aborts before
   editing anything if a note has no front matter, has other front-matter keys, is empty, or
   has a placeholder line (`TODO ...`, `TBD`, `DRAFT ...` at the start of a line, `[TODO]`,
   `<TBD>`);
7. regenerates `docs/CRATES.md` and the README crate table (`tools/gen-crate-index.py`, then
   its `--check`);
8. refreshes `Cargo.lock` with `cargo metadata --format-version 1 --offline`, which changes
   only the workspace members' own `version` lines (the MSRV-pinned third-party versions are
   kept; `cargo update --workspace` is never used). The script asserts the lock diff is only
   member versions (restoring the old lock otherwise) and then runs
   `cargo metadata --locked --offline`. `--no-lock` skips this step.

If apply stops on a note, nothing was edited: fix the note and re-run. `--no-finalize-notes`
returns to the old scaffold-and-edit flow.

Verify, read-only. Every row must PASS (the bindings row is INFO by design):

```bash
python3 tools/finalize-release-check.py $PAIRS
python3 tools/readme_snippets.py --check
python3 tools/gen-crate-index.py --check
python3 tools/prepare-release.py --restamp-check $(cat $PAIRS)
```

`--restamp-check` must report "already <today>" everywhere. If the wave spans midnight, run
`python3 tools/prepare-release.py --restamp $(cat $PAIRS)` and commit; the dates are a claim
about publication day.

Review `git diff --stat` (expect about 195 files: 54 manifests + dependants + READMEs +
CHANGELOGs + 56 notes + `Cargo.lock` + `docs/CRATES.md`) and read the `Cargo.lock` diff: only
`version = ` lines may differ.

### 1a. Private fixtures in the release lanes

Every `release*.yml` lane (all 51 per-crate lanes and the lockstep `release.yml`) checks
out the **pinned private-fixtures commit** through the shared composite action
`./.github/actions/private-fixtures`, with the read-only deploy key
(`secrets.PRIVATE_FIXTURES_DEPLOY_KEY`, the same one `ci.yml` already uses) and
`REQUIRE_PRIVATE_FIXTURES=1`. A missing private fixture is therefore a hard lane failure,
not a silent skip; fork repos have no secret, so there the action disables itself and the
real-capture tests skip loudly.

To **disable** the private fixtures (e.g. the key is revoked, or a private capture is
known bad while a fix is in flight), any one of:

* set the repository variable `SKIP_PRIVATE_FIXTURES` to `1` - Settings -> Secrets and
  variables -> Actions -> **Variables** tab -> New repository variable (this is what the
  lanes pass in as `skip: ${{ vars.SKIP_PRIVATE_FIXTURES }}`); or
* set a job-level `env: SKIP_PRIVATE_FIXTURES: "1"` in the lane; or
* locally, export `SKIP_PRIVATE_FIXTURES=1` before `cargo test`.

Locally the same variable makes the `test-fixtures` resolver treat the private tier as
absent (a loud `SKIPPED` line per fixture), so only the redistributable suite runs. The
skip **wins** over `REQUIRE_PRIVATE_FIXTURES=1` - that is deliberate, so disabling a
known-bad corpus cannot be defeated by the CI-only require flag.

#### Modes of the composite action (CI clones the private repo once per run)

The private repo is ~374 MB and GitHub jobs get separate runners, so `ci.yml` does the
clone **at most once per run**. The action's `mode` input selects the behaviour:

| `mode` | Used by | Behaviour |
| --- | --- | --- |
| `checkout` (default) | every `release*.yml` lane | exactly as before: pinned checkout into the calling job, `RUST_BROADCAST_PRIVATE` + `REQUIRE_PRIVATE_FIXTURES=1` |
| `prepare` | the single `private-fixtures` job in `ci.yml` | restores `private-fixtures-v1-<pinned sha>` from the Actions cache; only on a miss does it clone the pinned commit (sparse `fixtures`, lfs off) and save the cache |
| `use` | `test`, `test-no-default`, `dvb-stream` | restores the same cache key with `fail-on-cache-miss: true` (a miss is a job-ordering bug and FAILS), then points the tests at it |

Because the cache key is the pinned commit, a run whose `private` submodule pointer is
unchanged restores the cache and never clones at all; a changed pin clones once in
`private-fixtures` and every consumer restores it. The prep job never fails just because
the secret is absent (fork PRs): it reports `enabled=false`, succeeds, and the consumers
set `SKIP_PRIVATE_FIXTURES=1` so the private tests skip loudly. Consumers carry
`needs: private-fixtures`. `tools/tests/test_ci_private_once.py` pins this topology.

## 2. Gates on the bumped tree

One command per line, each exit status read:

```bash
RUSTFLAGS="-D warnings" cargo build --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo test --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo build --workspace --no-default-features --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo clippy -p dvb-ci-runtime --all-features --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings
python3 -m pytest tools/tests -q
python3 tools/check-published-dep-consistency.py --enforce-dev-cycles
```

The last is graph-only until crates go live (checks 1-2 compare the published manifests and
run blocking in CI on every tag). CI on the bump PR repeats all of it, including the MSRV
lane.

**Package check** (the lane gates test the repo tree; nothing else builds the tarball cargo
will actually publish, so a missing `include` path, a bad `readme` path or a verify-build
failure would first appear LIVE, mid-wave, on an immutable publish). It needs the bump
COMMITTED (cargo refuses a dirty tree), so run it on the merge commit in step 3, or in the CI
runner via the manual `Package check` workflow (`.github/workflows/package-check.yml`,
`workflow_dispatch`, run on the bump PR branch or `main`):

```bash
python3 tools/finalize-release-check.py $PAIRS --package
```

This is `cargo package --workspace --locked --exclude atsc3 --exclude test-fixtures
--exclude test-alloc --exclude test-bounded` (verify-builds every tarball) followed by a
sanity pass over each plan crate's `.crate`: under 10 MiB, `Cargo.toml` + the declared
`readme`/`license-file` + every literal `include` path present. Slow (about 2 minutes warm,
longer cold); it is never part of the default check. Cargo 1.95 verifies workspace members
against each other, so it works BEFORE any sibling version is on crates.io: it was run on a
scratch bump of this plan (all 54 crates, 2026-10-17) and passed. `--package-no-verify` skips
the verify-builds if a run ever needs it. Known warning, not a failure: the committed lock
pins `chacha20 0.10.1`, which crates.io marks yanked (`cargo package` prints it once per
crate that depends on it); decide on a lock update separately.

## 3. Bump PR and merge

```bash
git add -A
git commit -m "release: 2026-10 breaking wave (54 crates; broadcast-common 10, lockstep 12)"
git push -u origin release/2026-10
gh pr create --title "release: 2026-10 breaking wave" --body "Prepared with tools/prepare-release.py; finalize-release-check all PASS."
gh pr checks --watch
```

Merge only when **every** check is SUCCESS (not "no failures reported"). Then:

```bash
git switch main
git pull --ff-only
python3 tools/finalize-release-check.py $PAIRS
```

The check must pass again on the merge commit: tags are cut from exactly this commit, and
each lane reads its release note and manifest from the tagged tree. Record the SHA.

## 4. Create ALL tags locally first (no push)

Local tags cost nothing and let `push-release-tags.sh` refuse a typo before the first push.
Tag the merge commit only; never use `git push --tags`.

```bash
SHA=$(git rev-parse HEAD)
python3 tools/prepare-release.py --tag-order $(cat $PAIRS) > /tmp/release-tags.txt
wc -l /tmp/release-tags.txt        # 50
```

Nothing named like a planned tag may already exist on the remote. A stale remote tag makes
the later push a silent no-op (#933); `push-release-tags.sh` also refuses it, but find out
now. This must print nothing:

```bash
git ls-remote --tags origin $(sed 's#^#refs/tags/#' /tmp/release-tags.txt)
```

Create the tags, stopping at the first failure (a half-created set must not look complete):

```bash
for t in $(cat /tmp/release-tags.txt); do git tag "$t" "$SHA" || { echo "FAILED to create $t"; break; }; done
git tag --points-at "$SHA" | wc -l   # 50: counts only the tags on the release commit, not every tag in the repo
```

(`/tmp/release-tags.txt` lists the lockstep slot as `v12.0.0`, so that tag is created too.)
Preview the push order. This touches no git state and no network:

```bash
tools/push-release-tags.sh --dry-run --plan $PAIRS
```

Check it against section 6 of the release plan (generated from the same script, so they
agree): `broadcast-common-v10.0.0` first, `mp4-emsg` before `mpeg-ts`, the lockstep slot
(position 34 of 50, after `scte35-splice`, not right after `mpeg-ts`) marked `MANUAL`.

A real run (no `--dry-run`) preflights every tag against the TAGGED tree before pushing
anything: the local tag must point at `HEAD` (a pinned release commit that is not `HEAD` is
named with `--allow-tag-commit <sha>`), `git show <tag>:<crate>/Cargo.toml` must carry the
tag's version, `git show <tag>:docs/release-notes/<crate>-<ver>.md` must exist and not be
`status: DRAFT`, and no tag of that name may sit on the remote at another commit.

## 5. Push, one tag at a time

More than three tags in one push fires **no** workflows (#933). Never push in bulk.

```bash
tools/push-release-tags.sh --plan $PAIRS
```

For each tag, in order, the script: pushes that one tag, polls the crates.io sparse index
until the version appears, then waits for that tag's `Release*` workflow to finish. Its exit
status is truthful:

| exit | meaning | you do |
|---|---|---|
| 0 | every tag pushed or already live, every release workflow succeeded | continue to step 6 |
| 1 | a refusal, push failure, unreadable index, timeout, or a failed/cancelled workflow; nothing after it was pushed | read the output, fix, re-run the same command |
| 2 | stopped: a run is waiting on the `crates-io` approval gate, the plan reached `v12.0.0`, or a workspace dependency of the next tag is not live on crates.io at the version its tagged manifest requires (the output names it) | approve (below) / push the lockstep tag by hand / publish or wait for the named dependency, re-run the same command |

Before pushing each tag (plan and explicit-tag mode alike, and for the `v12.0.0` slot over
all five lockstep crates' external dependencies) the script reads the TAGGED manifest and
checks every workspace dependency (normal, build, target tables, versioned dev-deps) is on
the crates.io index at a version satisfying the manifest's requirement (max semver, yanked
versions ignored). Ordering is therefore no longer enforced only by the tag order: the lanes
themselves do not check their siblings, this script does, so **push only through it**.

Re-running is always safe: crates already live are skipped, and a tag already on the remote
is not pushed twice (it resumes waiting only when a release run exists for it; a remote tag
with no run is the #933 shape and stops the script).

### 5a. Approving the `crates-io` gate

Every release lane (`release.yml` and each `release-<crate>.yml`) runs `publish` with
`environment: crates-io`; the run sits in `waiting` until a required reviewer approves. To
approve from the CLI (document-only; run by the owner at release time):

```bash
gh run list --branch <tag> --workflow "Release <crate>" --limit 1 --json databaseId,status,url
gh api repos/fishloa/rust-broadcast/actions/runs/<run-id>/pending_deployments
gh api repos/fishloa/rust-broadcast/actions/runs/<run-id>/pending_deployments -X POST \
  -F 'environment_ids[]=<environment-id>' -f state=approved -f comment="release 2026-10"
```

The GET returns the `environment.id` for `crates-io`; the POST approves it. The lockstep
workflow is named `Release`; `release-stream.yml` is `Release dvb-stream`. The web UI
(run page -> "Review deployments") does the same.

### 5b. The lockstep tag, by hand

The plan reaches `v12.0.0` at position 34 of 50 (after `mpeg-ts-v0.6.0` and the rest of its
ready batch, up to `scte35-splice-v4.0.0`: the order is alphabetical within a ready batch);
the script stops there with exit 2 (`broadcast-common-v10.0.0` and `mpeg-ts-v0.6.0` must be
live first, and they are by then; it checks all five crates' external dependencies and says
"Do NOT push v12.0.0 yet" if one is missing).

```bash
git push origin refs/tags/v12.0.0
gh run list --branch v12.0.0 --workflow Release --limit 1 --json databaseId,status,url
```

Approve as in 5a. `release.yml` publishes `dvb-bbframe`, `dvb-si`, `dvb-conformance`,
`dvb-t2mi`, `dvb-tools` in that order. Wait until `dvb-tools` 12.0.0 is on the index:

```bash
python3 tools/crates-io.py check dvb-bbframe=12.0.0 dvb-si=12.0.0 dvb-conformance=12.0.0 dvb-t2mi=12.0.0 dvb-tools=12.0.0
```

then re-run `tools/push-release-tags.sh --plan $PAIRS`; it sees the lockstep slot live and
continues with `dvb-ci`, `dvb-stream`, ... to `multimux-cli`.

If a lane fails after a successful publish step (e.g. the GitHub Release step), re-running
the failed workflow is safe: publish is idempotent. If crates.io answers 429, wait and re-run
the lane.

## 6. Verify

After each push the script has already confirmed the index and the workflow. Confirm the
whole wave from the outside:

```bash
python3 tools/prepare-release.py --verify $(cat $PAIRS)
python3 tools/crates-io.py check $(cat $PAIRS)
python3 tools/check-published-dep-consistency.py --blocking --enforce-dev-cycles
```

**Expect red `Published-dependency consistency` runs.** That workflow
(`published-dep-consistency.yml`) runs `tools/check-published-dep-consistency.py --blocking` on
every tag push. While the wave is in flight the published manifests of the not-yet-published
crates are still on the OLD epoch, so it will go RED for tags of the 50 as they are pushed
(and stay red until the last dependent is live). It does NOT gate any release lane (lanes are
independent workflows), so a red run there is not a reason to stop; the only verdict that
counts is the final one in the block below, after everything is live.

`--verify` and `crates-io.py check` exit non-zero when a crate is not live at exactly the
planned version (a failed request is an error, never "absent"). Do not use hand-rolled
`curl` against crates.io: it 403s without a User-Agent.

docs.rs: use `status.json`, **not** `builds.json` (a 404). `doc_status: true` is green;
`false` means "still building" OR "failed" and is never reported as failed on its own:

```bash
for p in $(cat $PAIRS); do
  c=${p%%=*}; v=${p##*=}
  printf '%s %s ' "$c" "$v"
  curl -s -A "rust-broadcast-release (see Cargo.toml repository)" "https://docs.rs/crate/$c/$v/status.json"
  echo
done
```

Anything other than `{"doc_status":true,...}` is PENDING: re-poll after a few minutes. Treat
it as FAILED only with evidence: open `https://docs.rs/crate/<crate>/<ver>/builds`, take the
newest build and read its log for an error line. HTML instead of JSON means docs.rs has not
ingested the version yet (also PENDING).

GitHub Releases: each lane creates one from its note; open two or three and confirm the body
starts with the note title, not a YAML block. Attach `dvb-tools` binaries (its binaries job
is disabled in `release.yml`):

```bash
gh workflow run attach-binaries.yml -f tag=v12.0.0 -f crate=dvb-tools
```

`multimux-cli` does NOT ship its binary from its own lane either: that `binaries` job is
commented out in `release-multimux-cli.yml` (the `ubuntu-24.04-arm` runner causes
`startup_failure`), so attach it the same way once `multimux-cli-v0.11.0` is live:

```bash
gh workflow run attach-binaries.yml -f tag=multimux-cli-v0.11.0 -f crate=multimux-cli -f bin=multimux
```

Both binaries therefore need the manual `attach-binaries.yml` dispatch; confirm each
Release page lists its archives afterwards.

## 7. Post-publish

Bump the out-of-workspace consumers (they pin the old, now-superseded epochs; the
finalize check's INFO row listed them):

* `bindings/python/Cargo.toml` (`dvb-si`, `dvb-t2mi`: `"9"` today) -> `"12"`;
* `bindings/transmux-py/Cargo.toml` (`broadcast-common` `"9"`, `transmux` `"0.21"`) -> `"10"`,
  `"0.27"`.

Both are not workspace members and consume published versions, so this is a follow-up PR
after the crates are live. Regenerate each lock, run each binding's tests, then ship the
wheels through their own workflows (`python-bindings.yml`, `python-bindings-transmux.yml`);
those bump the bindings' own versions in the same PR.

Then:

* notify the downstream consumers: **zenith**, **zenith-tv** and **axis-origin**
  (`fishloa/axis-origin`, the extracted multimux ACAP packager). Point each at
  [`migration-notes-downstream.md`](migration-notes-downstream.md) (per-crate migration
  steps for the breaking epochs) and
  [`release-notes/2026-10-wave-overview.md`](release-notes/2026-10-wave-overview.md);
* update the CLAUDE.md "Unpublished" markers the audit flagged (owner edit), and the
  hand-edited README coverage items;
* run `python3 tools/finalize-release-check.py $PAIRS` once more on `main` (the bindings row
  should now read "ALREADY on new epoch" after the follow-up PR);
* delete `/tmp/release-tags.txt`.

## 8. Failure playbook

Ground rules: crates.io is immutable (a version can be yanked, never deleted or
overwritten); never `cargo publish` from a workstation, even to "finish" a wave; never move
a tag that has been pushed except in case (a) below; `push-release-tags.sh` stops at the
first failure and pushes nothing after it, so a failure is always "tag N of 50 failed, N+1..
not pushed". First find out what is true:

```bash
python3 tools/crates-io.py version <crate> ...   # what is live (a failed request is an error, never "absent")
gh run list --branch <tag> --json databaseId,status,conclusion,workflowName,url
gh run view <run-id> --log-failed                # was the publish step reached? "published <crate>" in the log?
```

**(a) A lane gate failed and NOTHING was published for that crate** (the index does not
show the version and the run never logged `published <crate>`; every gate runs before the
publish step). Fix forward on `main` by PR (fix, regenerate whatever the release prep
touched, re-run the gates). The tag still points at the broken commit and the lane builds
the TAGGED tree, so the tag has to move. Deleting a pushed tag is allowed **only** because
nothing shipped from it:

```bash
git push origin --delete <tag>       # remote first; the lane never published
git tag -d <tag>
git tag <tag> <fixed-sha>            # recreate at the fixed release commit (all tags must point at HEAD, or pass --allow-tag-commit)
tools/push-release-tags.sh --plan $PAIRS   # live crates are skipped, this one is pushed again
```

A re-created tag can fire `startup_failure` instead of a run (it is on the "Do not" list for
that reason). If that happens, stop re-using the tag name: bump that crate by a patch (b)
and tag the new version. If any crate in the lane DID publish before the failure, this
case does not apply: use (b) or (c).

**(b) A partial wave: some crates are live, one failed for a real reason.** Fix on `main`.
Bump ONLY the failed crate and the crates after it in the tag order that are not yet
published, with the semantics of the change (patch = fix only; minor = additive; major =
breaking, and a changed caret epoch of a sibling is major-class for its dependents; epoch
purity still applies). Already-live crates keep their versions: a requirement like `0.27`
is satisfied by `0.27.1`, so their dependents need no bump unless they need the fix (then
they raise their floor and take a patch too). Build a pair list of just those crates and
run the normal tools over it:

```bash
python3 tools/prepare-release.py --check <crate>=<new-version> ...
python3 tools/prepare-release.py <crate>=<new-version> ...
python3 tools/finalize-release-check.py <crate>=<new-version> ...
```

then PR, merge, tag the new versions at the new commit and run
`tools/push-release-tags.sh <crate>-v<new-version> ...` (or `--plan <subset-pairs-file>`; it
skips what is live and its dependency check refuses anything whose siblings are not).

**(c) A partial lockstep** (for example `dvb-si` 12.0.0 is live but `dvb-conformance`
failed for a real reason; the lockstep gate needs all five at ONE version and the crates
publish in the order `dvb-bbframe`, `dvb-si`, `dvb-conformance`, `dvb-t2mi`, `dvb-tools`).
If the failure was transient (429, runner hiccup), re-run the failed workflow on the same
tag: publish is idempotent ("already uploaded" counts as success), so already-live crates
are skipped. If the code or manifest was wrong, `12.0.0` of the live ones is immutable, so
**all five go to `12.0.1`** (`prepare-release.py` refuses fewer than all five): bump all five
together, write `docs/release-notes/v12.0.1.md`, tag `v12.0.1`, push it by hand (section
5b). `12.0.0` of the crates that did publish stays on crates.io; yank it only if it is
actually harmful (d).

**(d) Yank vs patch-republish.** Yank when a published version is dangerous to *new*
resolution: it does not build, is unsound, has a wrong public API that would mislead
dependents, or leaks something. A yank hides it from new resolves; existing lock files keep
it, and it is never deleted. A yank is always paired with a patch-republish (nothing else
gives resolvers a version to move to). Do not yank for a wrong CHANGELOG, doc typo, or
cosmetic metadata: those cannot be fixed in place; ship the correction in the next release.
Yanking is CI-only, through `yank.yml` (it uses the `CARGO_REGISTRY_TOKEN` secret):

```bash
gh workflow run yank.yml -f crates=dvb-si,dvb-t2mi -f version=12.0.0            # yank
gh workflow run yank.yml -f crates=dvb-si,dvb-t2mi -f version=12.0.0 -f undo=true  # un-yank
```

`crates` is a REQUIRED input with no default: the workflow used to default to
`dvb-common,dvb-si,dvb-t2mi,dvb-bbframe` (a crate that no longer exists, omitting the other
50), so a bare `-f version=...` yanked the wrong set. `push-release-tags.sh` ignores
yanked versions when deciding what is live (and `crates-io.py` reports the API's
`max_version`), so after a yank take that crate out of the pair list you re-run with, or the
script will treat it as unpublished and try to resume its tag.

**(e) Rate limits and the three-tag rule.** crates.io: a new version of an existing crate is
allowed a burst of 30 then about one a minute (new crates: 5 then one per 10 minutes, and no
crate of this wave is new). A 429 shows up as "failed to publish" in that lane's publish
step: wait out the `Retry-After` it prints, then "Re-run failed jobs" on that run (the
publish step is idempotent), then re-run `push-release-tags.sh`. GitHub: pushing more than
three tags in one push fires **no** workflows at all, silently (#933, and `git push --tags` is
the usual cause); the script pushes exactly one tag per step for that reason. A GitHub API
secondary rate limit (HTTP 403 from `gh`) while polling is not a failed release: wait a few
minutes and re-run the script; never "catch up" by pushing several tags. If a tag was pushed
and no run exists for it (the script says so, #933), and the crate is not live, delete that
remote tag and push it again on its own (case (a) commands; nothing was published).

## Do not

* push `atsc3-v0.1.0` (abandoned, never published; its lane fails safe by design);
* push `--tags`, or more than one tag at a time;
* edit a tag after pushing it (a re-created tag can fire `startup_failure`);
* run `cargo publish`, `gh release create` with artifacts, or any manual upload.
