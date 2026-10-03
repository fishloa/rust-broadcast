# De-hand-roll W0 — Dependency Bumps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take every outdated dependency that is not tied to a later wave, with no behaviour change, proven by the crypto/interop/oracle suites.

**Architecture:** One branch `w0/deps` in worktree `.worktree/w0`, with one commit per upgrade group so each can be reviewed and reverted on its own. API migrations are mechanical: follow each crate's CHANGELOG, and use the existing known-answer and interop tests as the correctness gate. No test may be edited except for the call sites of a renamed API.

**Tech Stack:** Rust 1.95 workspace, cargo `--locked`, RustCrypto 0.13 generation, rtc-* 0.21, base64 0.23, criterion 0.8, roxmltree 0.21.

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` (§4 SP0, §2 constraints). This plan is W0 of §7. W1–W3 get their own plans when the previous wave lands, because they build on its APIs.

## Global Constraints

- MSRV is **1.95.0**. `Cargo.lock` is committed; always build and test with `--locked`.
- A bump may change only the intended `Cargo.lock` entries. Restore any other drift with `cargo update -p <pkg> --precise <old>`.
- The workspace uses `resolver = "2"`, which is not MSRV-aware. Run every `cargo update` as `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update …` so that versions needing a newer rustc are not chosen.
- Target versions (all verified MSRV ≤ 1.89):
  - aes 0.9
  - ctr 0.10
  - cbc 0.2
  - cipher 0.5
  - hmac 0.13
  - sha1 0.11, sha2 0.11, md-5 0.11
  - pbkdf2 0.13
  - aes-kw 0.3
  - rtc-dtls/ice/shared/srtp/stun 0.21
  - base64 0.23
  - criterion 0.8
  - roxmltree 0.21
- **NOT in W0:**
  - axum, tower-http and reqwest (moved to W2/SP2)
  - sdp-types (moved to W1/SP4)
- No Co-Authored-By and no Claude-Session commit trailers.
- Never weaken, skip or delete a test. Known-answer vectors are the oracle; if one fails, the migration is wrong, not the vector.
- Never run two cargo commands concurrently.

## Review Focus

1. CENC `cenc`/`cbcs` and HLS Sample-AES (chained CBC, E-AC-3 IV reset per syncframe) ciphertext must be byte-identical. The independent oracle fixtures (Bento4/pycryptodome, `transmux/tests/fixtures/ORACLES.md`) must still pass. Owned by Task 2, step 5.
2. SRT key-wrap with 128/192/256-bit KEKs and PBKDF2-derived keys must match the vectors, and must interoperate with real libsrt. Owned by Task 2, step 5.
3. Digest MD5 responses and HMAC-SHA256 signed URLs in broadcast-auth must be unchanged. Owned by Task 2, step 5 (existing tests) plus the new pin test in Task 2, step 1.
4. DTLS fingerprint, STUN RFC 5769 and SRTP RFC 3711 vectors must pass after the rtc 0.21 bump. Owned by Task 3.
5. base64 decoding of real SCTE-35 and Digest/Basic inputs must be unchanged, including padding behaviour. Owned by Task 4, step 1.

---

### Task 0: Worktree setup

**Files:** none (environment only).

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w0/deps .worktree/w0 origin/main
cd .worktree/w0
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
```

- [ ] **Step 2: Baseline. The crypto, interop and vector suites must pass BEFORE any change**

```bash
timeout 1800 cargo test --locked --all-features -p srt-runtime -p transmux -p webrtc-runtime -p broadcast-auth -p scte35-splice 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Expected: only `test result: ok` lines. Record the passed-test counts in `.delegate/w0-report.md` under "baseline".

---

### Task 1: Semver-compatible updates

**Files:** Modify `Cargo.lock` only.

- [ ] **Step 1: Update within existing caret ranges, MSRV-aware**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p async-trait -p clap -p encoding_rs -p flate2 -p futures-core -p futures-util -p libc -p log -p rand@0.10 -p rustls -p thiserror@2 -p tokio-rustls
```

- [ ] **Step 2: Check the lock diff only touches those packages and their own transitive deps**

```bash
git diff Cargo.lock | grep -E '^[-+]name|^[-+]version' | paste - - | sort | uniq
```

Expected: only the packages above, plus any transitive dependencies they pull in. Note any other package change and its cause in `.delegate/w0-report.md`.

- [ ] **Step 3: Build and test the workspace**

```bash
cargo build --workspace --all-features --locked 2>&1 | grep -E '^(error|warning)' | head
timeout 5400 cargo test --workspace --all-features --locked --no-fail-fast 2>&1 | grep -E 'FAILED|panicked|^error' | head
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add Cargo.lock
git commit -m "chore(deps): take semver-compatible updates (rustls 0.23.45, tokio-rustls 0.26.6, thiserror 2.0.21, clap 4.6.7, …)"
```

---

### Task 2: RustCrypto 0.13 generation (one atomic group)

The crates share `cipher`/`digest`/`crypto-common` and must move together.

**Files:**
- Modify the Cargo.toml version requirements:
  - `srt-runtime/Cargo.toml`: aes, aes-kw, ctr, hmac, pbkdf2, sha1
  - `transmux/Cargo.toml`: aes, cbc, ctr
  - `webrtc-runtime/Cargo.toml`: aes, cipher, ctr, hmac, sha1, sha2
  - `broadcast-auth/Cargo.toml`: hmac, md-5, sha2
  - `multimux/Cargo.toml`: md-5
- Modify the call sites:
  - `srt-runtime/src/crypto.rs`
  - `transmux/src/cenc_crypto.rs`
  - `transmux/src/sample_aes.rs`
  - `webrtc-runtime/src/media/transport.rs`
  - `broadcast-auth/src/server.rs`
  - `broadcast-auth/src/signed_url.rs`
  - multimux md-5 users: `grep -rn 'md5::\|Md5' multimux/src`
- Test call sites (only if an API renamed):
  - `srt-runtime/tests/crypto_vectors.rs`
  - `webrtc-runtime/tests/srtp_rfc3711_vectors.rs`

**Interfaces:**
- Consumes: none.
- Produces: no public API change. Grep confirmed no RustCrypto type appears in any `pub` signature (`grep -rnE 'pub .*\b(Aes128|Hmac<|GenericArray|Ctr128BE|Sha256|Md5)\b' */src` is empty). The final step re-checks this.

- [ ] **Step 1: Add a pin test for the one crypto path without a known-answer vector**

The broadcast-auth Digest MD5 response and HMAC signed-URL signature must stay identical. Add this to `broadcast-auth/src/server.rs`'s existing `#[cfg(test)] mod tests`. It uses the RFC 2617 §3.5 worked example (user `Mufasa`, password `Circle Of Life`, realm `testrealm@host.com`, nonce `dcd98b7102dd2f0e8b11d0f600bfb0c093`, uri `/dir/index.html`, qop `auth`, nc `00000001`, cnonce `0a4f113b`, method GET → response `6629fae49393a05397450978507c4ef1`):

```rust
#[test]
fn digest_md5_matches_rfc2617_worked_example() {
    use md5::{Digest as _, Md5};
    let ha1 = hex_of(Md5::digest(b"Mufasa:testrealm@host.com:Circle Of Life"));
    let ha2 = hex_of(Md5::digest(b"GET:/dir/index.html"));
    let resp = hex_of(Md5::digest(
        format!("{ha1}:dcd98b7102dd2f0e8b11d0f600bfb0c093:00000001:0a4f113b:auth:{ha2}").as_bytes(),
    ));
    assert_eq!(resp, "6629fae49393a05397450978507c4ef1");
}

fn hex_of(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
```

Run it on the OLD versions first:

```bash
cargo test -p broadcast-auth --all-features --locked digest_md5_matches_rfc2617 2>&1 | grep -E 'test .*(ok|FAILED)'
```

Expected: PASS. Pinning the current behaviour is the point. Commit it on its own:

```bash
git add broadcast-auth/src/server.rs
git commit -m "test(broadcast-auth): pin Digest MD5 to the RFC 2617 §3.5 worked example"
```

- [ ] **Step 2: Bump the requirements**

Edit each manifest listed above, setting:
- `aes = "0.9"`, `ctr = "0.10"`, `cbc = "0.2"`, `cipher = "0.5"`
- `hmac = "0.13"`, `sha1 = "0.11"`, `sha2 = "0.11"`, `md-5 = "0.11"`
- `pbkdf2 = "0.13"`, `aes-kw = "0.3"`

Keep each manifest's existing column alignment, `default-features` and `features` keys. Then:

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p aes -p aes-kw -p ctr -p cbc -p cipher -p hmac -p sha1 -p sha2 -p md-5 -p pbkdf2
git diff Cargo.lock | grep -E '^[-+]name' | sort | uniq
```

Expected lock changes:
- the ten crates
- their shared bases: `cipher`, `digest`, `crypto-common`, `block-buffer`, `inout`
- the new `hybrid-array`, replacing `generic-array` where no other user remains

Nothing else may change.

- [ ] **Step 3: See what fails to compile**

```bash
cargo build --workspace --all-features --all-targets --locked 2>&1 | grep -E '^error' -A6 | head -80
```

- [ ] **Step 4: Migrate each call site using the upstream CHANGELOGs**

Read each changelog from the local registry, e.g.
`ls ~/.cargo/registry/src/*/cipher-0.5*/CHANGELOG.md`; repeat for aes, ctr, cbc, hmac, digest, pbkdf2 and aes-kw. Apply the documented renames. The known 0.4→0.5 generation changes:
- `generic_array::GenericArray` → `hybrid_array::Array` (re-exported as `cipher::Array` / `aes::cipher::Array`):
  - `GenericArray::from_slice(s)` → `Array::try_from(s)` / `<&Array<_, _>>::try_from(s)`, with the `?`/`expect` matching how the old code handled length
  - `GenericArray::clone_from_slice` → `Array::try_from(s)`
- Block traits:
  - `BlockEncrypt`/`BlockDecrypt` → `BlockCipherEncrypt`/`BlockCipherDecrypt`
  - `BlockEncryptMut`/`BlockDecryptMut` (cbc) → `BlockModeEncrypt`/`BlockModeDecrypt`
  - `KeyInit`, `KeyIvInit` and `StreamCipher` keep their names
- hmac: `Mac::new_from_slice`, `update` and `finalize().into_bytes()` are unchanged; `hmac::digest::Key` → `hmac::digest::Key` re-export, so check.
- pbkdf2 0.13: `pbkdf2::pbkdf2::<Hmac<Sha1>>(pw, salt, rounds, out)` returns `Result`. Keep the existing error mapping.
- aes-kw 0.3: `KekAes128::new(&key)` + `wrap`/`unwrap` → see its changelog. The output buffer API changed; keep the same output bytes.

Port mechanically and change no logic. If a call needs a semantic decision (e.g. length handling), keep the old behaviour exactly and note it in `.delegate/w0-report.md`.

- [ ] **Step 5: Run the oracle suites. All must pass unmodified**

```bash
timeout 1800 cargo test --locked --all-features -p srt-runtime -p transmux -p webrtc-runtime -p broadcast-auth 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
timeout 900 cargo test --locked --all-features -p srt-runtime --test crypto_vectors --test libsrt_interop -- --nocapture 2>&1 | grep -E 'test .*(ok|FAILED)|SKIP' | head -40
timeout 900 cargo test --locked --all-features -p transmux cenc sample_aes 2>&1 | grep -E 'test result|FAILED'
```

Expected:
- the same passed counts as the Task 0 baseline
- `crypto_vectors` and `libsrt_interop` all `ok`, not SKIPped (libsrt is installed locally)
- the CENC and Sample-AES oracle tests `ok`

- [ ] **Step 6: Re-check that no crypto type leaked into a public API**

```bash
grep -rnE 'pub .*\b(Aes128|Aes192|Aes256|Hmac<|Array<|GenericArray|Ctr128BE|Sha256|Sha1|Md5|Cbc)\b' --include='*.rs' */src | grep -v '^\S*:[0-9]*:\s*//' | head
```

Expected: no output. If there is output, record the crate in `.delegate/w0-report.md` as **breaking** (for the epoch/semver rule, spec §2).

- [ ] **Step 7: Commit**

```bash
git add srt-runtime transmux webrtc-runtime broadcast-auth multimux/Cargo.toml Cargo.lock
git commit -m "chore(deps): RustCrypto 0.13 generation (aes 0.9, ctr 0.10, cbc 0.2, cipher 0.5, hmac 0.13, sha1/sha2/md-5 0.11, pbkdf2 0.13, aes-kw 0.3)"
```

---

### Task 3: rtc-* 0.21 (webrtc-runtime)

**Files:**
- Modify `webrtc-runtime/Cargo.toml`: rtc-dtls, rtc-ice, rtc-shared, rtc-srtp, rtc-stun → `"0.21"`.
- Modify `webrtc-runtime/src/media/gather.rs` and `webrtc-runtime/src/media/transport.rs`.
- Test call sites only if an API renamed:
  - `webrtc-runtime/tests/dtls_fingerprint.rs`
  - `webrtc-runtime/tests/srtp_rfc3711_vectors.rs`
  - `webrtc-runtime/tests/stun_rfc5769_vectors.rs`
  - `webrtc-runtime/tests/whip_smoke_pcap_stun.rs`

**Interfaces:**
- Produces: no change to webrtc-runtime's public API. Re-check with `grep -rn 'pub .*rtc_' webrtc-runtime/src`. Any hit means a breaking change for webrtc-runtime; record it.

- [ ] **Step 1: Bump and update**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p rtc-dtls -p rtc-ice -p rtc-shared -p rtc-srtp -p rtc-stun
git diff Cargo.lock | grep -E '^[-+]name' | sort | uniq
```

- [ ] **Step 2: Build and migrate**

```bash
cargo build -p webrtc-runtime --all-features --all-targets --locked 2>&1 | grep -E '^error' -A6 | head -60
```

Fix each error using the rtc-* 0.21 changelog or release notes (`ls ~/.cargo/registry/src/*/rtc-*-0.21*/`). Port mechanically.

- [ ] **Step 3: Vector and interop suites**

```bash
timeout 900 cargo test -p webrtc-runtime --all-features --locked 2>&1 | grep -E '^test result|FAILED|panicked'
timeout 1800 cargo test -p multimux --all-features --locked --test whip_ingest --test whep_egress 2>&1 | grep -E '^test result|FAILED|panicked'
```

Expected: all ok, with the same counts as the baseline.

- [ ] **Step 4: Commit**

```bash
git add webrtc-runtime Cargo.lock
git commit -m "chore(deps): rtc-dtls/ice/shared/srtp/stun 0.21"
```

---

### Task 4: base64 0.23

**Files:**
- Modify the manifests `broadcast-auth/Cargo.toml`, `multimux/Cargo.toml` and `scte35-splice/Cargo.toml` (dev): `base64 = "0.23"`.
- Modify the call sites:
  - `broadcast-auth/src/server.rs`
  - `broadcast-auth/src/signed_url.rs`
  - `multimux/src/origin/mod.rs`
  - `multimux/src/output/whep.rs`
  - tests: `multimux/tests/rtsp_ingest.rs`, `scte35-splice/tests/{dvb_ta,known_vectors,serde_round_trip,spec_samples}.rs`

- [ ] **Step 1: Pin padding behaviour before the bump**

Add this to `broadcast-auth/src/server.rs`'s tests module. Basic credentials from RFC 7617 §2: `Aladdin:open sesame` ↔ `QWxhZGRpbjpvcGVuIHNlc2FtZQ==`.

```rust
#[test]
fn basic_credentials_base64_matches_rfc7617_example() {
    use base64::Engine as _;
    let enc = base64::engine::general_purpose::STANDARD.encode(b"Aladdin:open sesame");
    assert_eq!(enc, "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    let dec = base64::engine::general_purpose::STANDARD
        .decode("QWxhZGRpbjpvcGVuIHNlc2FtZQ==")
        .unwrap();
    assert_eq!(dec, b"Aladdin:open sesame");
}
```

```bash
cargo test -p broadcast-auth --all-features --locked basic_credentials_base64 2>&1 | grep -E 'test .*(ok|FAILED)'
git add broadcast-auth/src/server.rs && git commit -m "test(broadcast-auth): pin Basic base64 to the RFC 7617 §2 example"
```

Expected: PASS.

- [ ] **Step 2: Bump, migrate, test**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p base64@0.22
cargo build --workspace --all-features --all-targets --locked 2>&1 | grep -E '^error' -A6 | head -40
```

The `base64` 0.23 changelog (`~/.cargo/registry/src/*/base64-0.23*/RELEASE-NOTES.md`) lists the API changes. Apply them mechanically.

```bash
timeout 1800 cargo test --locked --all-features -p broadcast-auth -p scte35-splice -p multimux 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Expected: all ok.

Note: other crates may still pull base64 0.22 transitively, e.g. via reqwest. Two versions in the lock is expected until W2.

- [ ] **Step 3: Commit**

```bash
git add broadcast-auth multimux scte35-splice Cargo.lock
git commit -m "chore(deps): base64 0.23"
```

---

### Task 5: criterion 0.8 (benches only)

**Files:**
- Modify the manifests `broadcast-common`, `dvb-bbframe`, `dvb-csa`, `dvb-si`, `dvb-t2mi` (`[dev-dependencies] criterion = "0.8"`).
- Modify the benches:
  - `broadcast-common/benches/crc32.rs`
  - `dvb-bbframe/benches/bbframe_hot_paths.rs`
  - `dvb-csa/benches/throughput.rs`
  - `dvb-si/benches/si_hot_paths.rs`
  - `dvb-t2mi/benches/t2mi_hot_paths.rs`

- [ ] **Step 1: Bump and compile the benches**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p criterion
cargo build --locked --all-features --benches -p broadcast-common -p dvb-bbframe -p dvb-csa -p dvb-si -p dvb-t2mi 2>&1 | grep -E '^(error|warning)' -A5 | head -40
```

Migrate per the criterion 0.6/0.7/0.8 changelogs. The known change is that `criterion::black_box` is deprecated in favour of `std::hint::black_box`, so switch to `std::hint::black_box`.

- [ ] **Step 2: Clippy on the benches (they are `--all-targets` in the gate)**

```bash
cargo clippy --locked --all-features --all-targets -p broadcast-common -p dvb-bbframe -p dvb-csa -p dvb-si -p dvb-t2mi -- -D warnings 2>&1 | grep -E '^(error|warning)' | head
```

Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add broadcast-common dvb-bbframe dvb-csa dvb-si dvb-t2mi Cargo.lock
git commit -m "chore(deps): criterion 0.8 for benches"
```

---

### Task 6: roxmltree 0.21 (atsc3 only)

**Files:** Modify `atsc3/Cargo.toml` (`roxmltree = "0.21"`) and `atsc3/src/slt.rs` and `atsc3/src/error.rs` only if the API changed.

- [ ] **Step 1: Bump, build, test**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p roxmltree
cargo build -p atsc3 -p atsc3-route --all-features --all-targets --locked 2>&1 | grep -E '^error' -A5 | head
cargo build -p atsc3 --no-default-features --locked 2>&1 | grep -E '^error' | head
timeout 600 cargo test -p atsc3 --all-features --locked 2>&1 | grep -E '^test result|FAILED'
```

Expected: no errors, tests ok.

- [ ] **Step 2: Commit**

```bash
git add atsc3 Cargo.lock
git commit -m "chore(deps): roxmltree 0.21 (atsc3)"
```

---

### Task 7: Version recording and full gate

**Files:** Modify `.delegate/release-versions.txt` (only if Task 2 step 6 or Task 3 found a public exposure), `.delegate/w0-report.md`.

- [ ] **Step 1: Full gate on the branch**

```bash
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" > /Volumes/External/Projects/rust-broadcast/.delegate/gate-w0.log 2>&1
grep -c '^rc=0' /Volumes/External/Projects/rust-broadcast/.delegate/gate-w0.log
grep -B1 -A10 '^rc=[1-9]' /Volumes/External/Projects/rust-broadcast/.delegate/gate-w0.log | head -30
```

Expected: `14` and no failure block.

- [ ] **Step 2: Confirm the outdated list is now down to only the later-wave items**

```bash
python3 /private/tmp/claude-501/-Volumes-External-Projects-rust-broadcast/afee462d-1b95-42c9-a99a-a771bfef51c0/scratchpad/outdated.py 2>&1 | tail -15
```

Expected remaining entries: axum, tower-http, reqwest (W2), sdp-types (W1), and roxmltree only if another user still pins 0.20. If the script is gone, re-derive the list with `cargo tree --workspace --depth 1 --locked` against crates.io via `tools/crates-io.py version <crate>`.

- [ ] **Step 3: Write `.delegate/w0-report.md`**

It contains:
- baseline vs after test counts per crate
- every lock change outside the intended set, with its cause
- every semantic decision taken during a migration
- the public-exposure check results

- [ ] **Step 4: Hand off for review**

Do not merge. The orchestrator runs the adversarial reviewer, then merges with `git merge --squash`, pushes and confirms CI green.
