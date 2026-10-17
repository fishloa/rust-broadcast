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
  environment (step 7).

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

```bash
for t in $(cat /tmp/release-tags.txt); do git tag "$t" "$SHA"; done
git tag -l | wc -l                 # includes all 50 new tags
```

(`/tmp/release-tags.txt` lists the lockstep slot as `v12.0.0`, so that tag is created too.)
Preview the push order. This touches no git state and no network:

```bash
tools/push-release-tags.sh --dry-run --plan $PAIRS
```

Check it against section 6 of the release plan: `broadcast-common-v10.0.0` first,
`mp4-emsg` before `mpeg-ts`, the lockstep slot after `mpeg-ts` and marked `MANUAL`.

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
| 2 | stopped: a run is waiting on the `crates-io` approval gate, or the plan reached `v12.0.0` | approve (below) / push the lockstep tag by hand, re-run the same command |

Re-running is always safe: crates already live are skipped, and a tag already on the remote
is not pushed twice.

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

The plan reaches `v12.0.0` right after `mpeg-ts-v0.6.0`; the script stops there with exit 2
(`broadcast-common-v10.0.0` and `mpeg-ts-v0.6.0` must be live first, and they are by then).

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

`multimux-cli` ships its binary from its own lane.

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

* tell downstream consumers: point them at
  [`migration-notes-downstream.md`](migration-notes-downstream.md) and
  [`release-notes/2026-10-wave-overview.md`](release-notes/2026-10-wave-overview.md);
* update the CLAUDE.md "Unpublished" markers the audit flagged (owner edit), and the
  hand-edited README coverage items;
* run `python3 tools/finalize-release-check.py $PAIRS` once more on `main` (the bindings row
  should now read "ALREADY on new epoch" after the follow-up PR);
* delete `/tmp/release-tags.txt`.

## Do not

* push `atsc3-v0.1.0` (abandoned, never published; its lane fails safe by design);
* push `--tags`, or more than one tag at a time;
* edit a tag after pushing it (a re-created tag can fire `startup_failure`);
* run `cargo publish`, `gh release create` with artifacts, or any manual upload.
