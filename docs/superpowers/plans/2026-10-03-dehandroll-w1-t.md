# De-hand-roll W1-T — transmux, timed-metadata, scte35-splice, broadcast-common, broadcast-auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every generic hand-rolled protocol/format site owned by cluster T (spec §3, §7 row T) with an established crate: `http-auth`/`headers`/`lru`/`url`/`form_urlencoded` in broadcast-auth, `url` for BaseURL resolution in transmux, `sdp-types` 0.2 for transmux RTP SDP, `base64`, `hex` and `jiff` across the five crates. Fix defect 6 (signed-URL `kid` not percent-encoded) and every real defect found on the way, each with a revert-checked regression test.

**Architecture:** One branch `w1/t` in worktree `.worktree/w1-t`, one commit per task so each can be reviewed and reverted alone. Goldens for every touched wire output are generated from unmodified `main` code FIRST (Tasks 2–4) and compared byte-for-byte afterwards; a deliberate difference is edited into the golden in the SAME commit as the change and listed in the CHANGELOG with an example. Public API of `broadcast_common::hex`, `transmux::rtp::{base64_encode, base64_decode, hex_decode}` and `scte35_splice::dvb_ta::base64_encode` is kept (they become one-line delegates), so only the sites the spec lists as breaking break. The guard tripwire tests (spec §5) land in the last code task.

**Tech Stack:** Rust 1.95 workspace, `--locked` everywhere. New dependencies (all MSRV-checked in spec §2 except `url`, `form_urlencoded` and `headers`, whose manifests declare 1.63, 1.51 and 1.57; `http-auth` 1.70, `sdp-types` 1.71, `base64` 0.23 1.71, all read from the crate manifests):
- `http-auth` 0.1.10 (already used), `headers` 0.4.2, `lru` 0.18.5, `url` 2.5.8 (in lock), `form_urlencoded` 1.2.2 (in lock) — broadcast-auth
- `url` 2.5.8, `sdp-types` 0.2.0, `jiff` 0.2.37, `base64` 0.23.1, `hex` 0.4.3 — transmux
- `jiff` (no_std+alloc), `hex` — timed-metadata; `base64`, `hex` — scte35-splice; `hex` — broadcast-common

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` — §1 decisions, §2 constraints, §3 inventory (HTTP/URL/SDP/encodings/time rows for these crates + defect 6), §4 SP2.4, SP3, SP4, SP5, §5 guards, §6 verification, §7 row W1-T, §8 versioning, §9 exceptions. This plan is W1 cluster **T** of §7. Upstream APIs named here were read from `~/.cargo/registry/src/*/` (http-auth 0.1.10, base64 0.23.1, hex 0.4.3, url 2.5.8, form_urlencoded 1.2.2) or from crates downloaded into the session scratchpad (headers 0.4.2, lru 0.18.5, jiff 0.2.37, sdp-types 0.2.0).

## Global Constraints

Spec §2, verbatim:
- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95: verified for all 34 bumps (max 1.89) and for the new crates (`tokio-util` 1.71, `socket2` 1.70, `backon` 1.85, `parking_lot` 1.71, `lru` 1.85, `jiff` 1.70, `hex`, `base64`, `arc-swap`, `wait-timeout`).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API, that crate takes a major-class version change. Each wave records this in `.delegate/release-versions.txt`.

Owner decisions that apply to this cluster:
- Q1/Q2: `no_std` may be dropped, but only in the crates this work touches and only where needed. Pure parser crates stay `no_std`. `broadcast-common` MUST stay `no_std`-capable (`hex` is used with `default-features = false, features = ["alloc"]`). `timed-metadata` and `scte35-splice` stay `no_std`+`alloc` (jiff/base64/hex all support it). Only the transmux RTP SDP builder becomes `std`-only (SP4).
- SP3: URL references with no absolute base use option (a): a fixed synthetic base whose prefix is stripped from relative results through ONE tested function, or the MPD's own/file URL.
- SP4: fmtp parameter lists stay codec logic (built as a string, never through sdp-types' typed `Fmtp`, whose `Display` joins with `;` and would change the bytes).
- SP5: dates and durations use `jiff`; the optional `chrono` features are untouched.
- `broadcast_common::hex` keeps its public API and delegates (NOT breaking).

Cluster rules:
- Never run two cargo commands concurrently. Never `git add -A`; add exact paths. Commit messages have no trailers.
- Do NOT bump any `version =` in a Cargo.toml and do not edit `.delegate/release-versions.txt` — version notes go in `.delegate/w1-t-report.md` (the orchestrator records them).
- Another W1 cluster (R-low) bumps `sdp-types` to 0.2 via rtsp-runtime, and others touch `Cargo.lock`. Expect a trivial `Cargo.lock` conflict at merge; do not pre-empt it. This branch may carry both `sdp-types` 0.1.8 (rtsp-runtime) and 0.2.0 (transmux) until R-low merges.
- A public API change in this cluster that breaks multimux gets the minimal call-site fix in the same task. Current analysis: multimux uses `transmux::rtp::base64_encode` (kept), `broadcast_auth::{Verifier, SignedUrlKeySet}` (kept), and none of the removed APIs (`transmux::uri`, `build_sdp_with_connection`, `format_rfc3339_ms`). Every task that changes a public item ends with `cargo check -p multimux --all-features --locked`; expected clean.
- Dependency-add procedure (used by every task that adds a crate; `--locked` fails until the lock has the entry): add the line to the manifest, run `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo metadata --format-version 1 >/dev/null`, then `git diff Cargo.lock | grep -E '^[-+]name|^[-+]version' | paste - - | sort -u` — only the intended new packages (and their own transitive deps) may appear; restore anything else with `cargo update -p <pkg> --precise <old>`. Commit the lock together with the manifest.
- Revert-check recipe (used by every defect task): after the task's commit, temporarily reintroduce the OLD behaviour with the exact edit named in the task, run the named test, paste the FAIL output into `.delegate/w1-t-report.md` under "Revert-check evidence", then `git checkout -- <file>` and re-run to see PASS. `git status` must be clean afterwards. Never claim a mutation that was not recompiled and re-run.

## Review Focus

Five inputs most likely to bite users that no existing test would catch. Each has a test in the named task.
1. **Digest `Authorization` strictness after moving to `http-auth`'s `ChallengeParser`** (Task 10): a client whose `username` has a `"` or `\` (the http-auth client escapes them as quoted-pairs, which the old splitter mis-parsed), a header repeating `username=` (old code: last wins — a smuggling vector; new: reject), upper-cased parameter names (RFC 7235: names are case-insensitive), and a raw non-ASCII `username` (the parser is ASCII-only; browsers send raw UTF-8 usernames; this is a deliberate, CHANGELOG-listed behaviour change — RFC 7616 §3.4 wants `username*`/`userhash` for non-ASCII). Tests: `broadcast-auth/tests/digest_params.rs`.
2. **base64 decode leniency in SDP `sprop-parameter-sets`** (Task 7): the hand-rolled decoder accepted missing padding, stray `=`, and non-zero trailing bits; real cameras emit unpadded sprop. The `base64` crate's default is strict, so the shim uses `PAD_INDIFFERENT` + `allow_trailing_bits`. Tests: unpadded real ffmpeg sprop → byte-identical avcC; `Zh==`; RFC 4648 §10 vectors.
3. **digest-uri substitution guard must not be loosened by URL normalisation** (Task 10b): `url::Url::parse` removes `.`/`..` segments, lower-cases hosts and drops default ports, so a naive "parse then compare" would let a response hashed over `http://h/a/../b` authorise `/b` (and `%2e%2e` likewise). The new rule requires the client's `uri` to already be in normalised form (`Url::as_str() == client_uri`) AND its path+query to equal the request-target exactly. Tests: in-module `digest_uri_matches_*`.
4. **BaseURL resolution edge cases** (Task 13): `..` climbing above the synthetic base (returns an absolute path, listed difference), an absolute-path reference vs the stripped synthetic base (`/x` must stay `/x`), non-ASCII references (now percent-encoded by `url`), CR/LF/tab (the WHATWG parser silently DELETES them, so the CR/LF guard must run BEFORE `Url::join`), and `\` (treated as `/` for special schemes). Tests: `transmux/tests/base_url.rs`.
5. **Signed-URL compatibility** (Task 9): a `kid` with `& = % +` or space (defect 6), IPv6 `ip=` (now percent-encoded `%3A` on the wire — listed difference), a URL minted by the OLD code with a literal `+` in the `kid` (now decodes to a space: rejected — listed breaking difference), duplicate `kid=` params (first wins, as before), and a bare `ip` key with no `=` (old: ignored, i.e. "no IP binding"; new: rejected — stricter). Tests: `broadcast-auth/tests/signed_url_kid.rs`.

Honourable mentions covered by tests but not in the top five: `timed-metadata`'s `parse_hex` panics on a multi-byte character and accepts `+` (Task 6b); transmux CLI `parse_hex16` panics on non-ASCII and accepts `+` (Task 6a); Bearer token CR/LF header injection (Task 10c); `WWW-Authenticate` realm `"`/CR/LF injection (Task 10d); `format_rfc3339_ms` out-of-range epoch and `media_to_epoch_ms` i64 overflow (Task 11).

---

### Task 0: Worktree setup and baseline

**Files:** none (environment) plus create `.delegate/w1-t-report.md`.

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w1/t .worktree/w1-t origin/main
cd .worktree/w1-t
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
```

- [ ] **Step 2: Baseline. Every suite of the cluster must pass BEFORE any change**

```bash
timeout 3600 cargo test --locked --all-features -p broadcast-common -p broadcast-auth -p scte35-splice -p timed-metadata -p transmux 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
cargo build --locked --no-default-features -p broadcast-common -p scte35-splice -p timed-metadata -p transmux 2>&1 | tail -3
cargo check --locked --all-features -p multimux 2>&1 | tail -3
```

Expected: only `test result: ok` lines; both builds finish with `Finished`. Record the per-binary passed counts in `.delegate/w1-t-report.md` under the heading `## Baseline` (create the file with a `# W1-T report` title first). A failure here is a pre-existing break: STOP and report it, do not "fix" it in this branch.

- [ ] **Step 3: Record the baseline commit**

```bash
git rev-parse HEAD
```

Write that hash into the report under `## Baseline` — it is the commit all goldens are generated from ("main before the wave"). No commit in this task (the report file is committed with Task 16).

---

### Task 1: SP2.4 verification gate — does `http-auth` 0.1.10 parse RFC 7616 credentials and render a challenge?

Spec §4 SP2.4 / §10 make this the first task. Findings from reading `~/.cargo/registry/src/*/http-auth-0.1.10/src/`:
- `parser.rs`: `ChallengeParser` is an RFC 7235 `challenge` parser (`scheme` + comma-separated `auth-param`s). It rejects `token68` ("Doesn't allow `token68`") and non-ASCII bytes. `Authorization: Digest …` credentials use the same `auth-param` grammar, so it should parse; `Authorization: Basic <base64>` is `token68` and should NOT.
- `lib.rs:117-151`: `ChallengeRef` implements only `Debug` (`grep -c 'impl.*Display' lib.rs` is 0); `ParamValue` likewise (`lib.rs:678`). There is no serializer anywhere in the crate, so "render WWW-Authenticate via `ChallengeRef`'s Display" is not possible.
- `digest.rs:543-570` (`append_quoted_key_value`) shows the client escapes `"`/`\` as quoted-pairs in values, which the old server splitter (`server.rs:673`, "No backslash-escape handling") cannot read.

**Files:**
- Create: `broadcast-auth/tests/http_auth_rfc7616.rs`
- Modify: `.delegate/w1-t-report.md`

**Interfaces:** none (verification only). Its outcome decides Tasks 10a–10d:
- parse OK for Digest + `Display` absent + token68 rejected (expected) ⇒ Digest fields via `ChallengeParser` (10a), Basic/Bearer via `headers::Authorization` (10c = the spec's own fallback, scoped to Basic/Bearer), challenge rendering stays a formatter but gains quoted-string escaping and CR/LF safety (10d) — escalation **E1**.
- If ANY RFC 7616 credential in step 2 fails to parse ⇒ the spec fallback applies to Digest too: skip 10a, keep `split_digest_fields`, write escalation **E0** (Digest gap) in the report, still do 10b–10d.

- [ ] **Step 1: Write the verification test**

```rust
//! SP2.4 verification gate: can `http-auth`'s `ChallengeParser` read RFC 7616
//! `Authorization: Digest` credentials, and does it round-trip our own
//! `WWW-Authenticate` challenge? Pins the facts the rest of the migration
//! depends on (see the plan, Task 1).

use broadcast_auth::{Credentials, Verifier};
use http_auth::{ChallengeParser, ChallengeRef};

/// RFC 7616 §3.9.1 (also http-auth's own `digest.rs` test vector), MD5.
const RFC7616_MD5: &str = "Digest username=\"Mufasa\", realm=\"http-auth@example.org\", \
    uri=\"/dir/index.html\", algorithm=MD5, \
    nonce=\"7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v\", nc=00000001, \
    cnonce=\"f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ\", qop=auth, \
    response=\"8ca523f5e9506fed4657c9700eebdbec\", \
    opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\"";

/// RFC 7616 §3.9.1, SHA-256 variant.
const RFC7616_SHA256: &str = "Digest username=\"Mufasa\", realm=\"http-auth@example.org\", \
    uri=\"/dir/index.html\", algorithm=SHA-256, \
    nonce=\"7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v\", nc=00000001, \
    cnonce=\"f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ\", qop=auth, \
    response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\", \
    opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\"";

fn parse_one(header: &str) -> ChallengeRef<'_> {
    let mut all = ChallengeParser::new(header)
        .collect::<Result<Vec<_>, _>>()
        .expect("credentials must parse as one RFC 7235 auth-param list");
    assert_eq!(all.len(), 1, "credentials are a single challenge-shaped list");
    all.remove(0)
}

fn param(c: &ChallengeRef<'_>, name: &str) -> Option<String> {
    c.params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.to_unescaped())
}

#[test]
fn rfc7616_md5_credentials_parse() {
    let c = parse_one(RFC7616_MD5);
    assert!(c.scheme.eq_ignore_ascii_case("Digest"));
    assert_eq!(c.params.len(), 10);
    assert_eq!(param(&c, "username").as_deref(), Some("Mufasa"));
    assert_eq!(param(&c, "realm").as_deref(), Some("http-auth@example.org"));
    assert_eq!(param(&c, "uri").as_deref(), Some("/dir/index.html"));
    assert_eq!(param(&c, "algorithm").as_deref(), Some("MD5"));
    assert_eq!(param(&c, "nc").as_deref(), Some("00000001"));
    assert_eq!(param(&c, "qop").as_deref(), Some("auth"));
    assert_eq!(
        param(&c, "response").as_deref(),
        Some("8ca523f5e9506fed4657c9700eebdbec")
    );
    assert_eq!(
        param(&c, "nonce").as_deref(),
        Some("7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v")
    );
}

#[test]
fn rfc7616_sha256_credentials_parse() {
    let c = parse_one(RFC7616_SHA256);
    assert_eq!(param(&c, "algorithm").as_deref(), Some("SHA-256"));
    assert_eq!(
        param(&c, "response").as_deref(),
        Some("753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1")
    );
}

/// RFC 7616 §3.4: the extended `username*` parameter (RFC 5987 encoding).
#[test]
fn username_star_extended_parameter_parses() {
    let c = parse_one("Digest username*=UTF-8''J%C3%A4s%20Sch%C3%B6n, realm=\"r\"");
    assert_eq!(
        param(&c, "username*").as_deref(),
        Some("UTF-8''J%C3%A4s%20Sch%C3%B6n")
    );
}

/// A quoted-pair inside a quoted-string is unescaped (the old splitter had no
/// backslash handling at all).
#[test]
fn quoted_pairs_are_unescaped() {
    let c = parse_one(r#"Digest username="a\"b\\c", realm="r""#);
    assert_eq!(param(&c, "username").as_deref(), Some(r#"a"b\c"#));
}

/// `Basic` credentials are `token68`, which the parser documents it does not
/// support — so Basic/Bearer need another reader (`headers::Authorization`).
#[test]
fn basic_token68_credentials_are_not_parseable() {
    let r = ChallengeParser::new("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==").collect::<Result<Vec<_>, _>>();
    assert!(r.is_err(), "token68 must be rejected: {r:?}");
}

/// The parser is ASCII-only (its doc: "Doesn't allow non-ASCII characters").
/// The old hand parser accepted a raw UTF-8 username; this pins the
/// behaviour change Task 10a lists in the CHANGELOG.
#[test]
fn raw_non_ascii_quoted_value_is_a_parse_error() {
    let r = ChallengeParser::new("Digest username=\"Jäs\", realm=\"r\"").collect::<Result<Vec<_>, _>>();
    assert!(r.is_err(), "{r:?}");
}

/// Our own server challenge is a well-formed RFC 7235 challenge: it is the
/// oracle Task 10d keeps using after the renderer gains escaping.
#[test]
fn own_digest_challenge_parses_as_one_challenge() {
    let v = Verifier::new(
        Credentials::Digest {
            username: "admin".into(),
            password: "12345".into(),
        },
        "cameras",
    );
    let header = v.challenge();
    let c = parse_one(&header);
    assert_eq!(c.scheme, "Digest");
    assert_eq!(param(&c, "realm").as_deref(), Some("cameras"));
    assert_eq!(param(&c, "qop").as_deref(), Some("auth"));
    assert_eq!(param(&c, "algorithm").as_deref(), Some("MD5"));
    assert_eq!(param(&c, "nonce").unwrap().len(), 96); // 48 raw bytes, hex
}
```

- [ ] **Step 2: Run it**

```bash
cargo test -p broadcast-auth --all-features --locked --test http_auth_rfc7616 2>&1 | grep -E 'test |test result|panicked'
grep -c 'impl.*Display' ~/.cargo/registry/src/*/http-auth-0.1.10/src/lib.rs
grep -n 'Debug for ChallengeRef' ~/.cargo/registry/src/*/http-auth-0.1.10/src/lib.rs
```

Expected: 7 tests `ok`; the `grep -c` prints `0` (no `Display` for `ChallengeRef`/`ParamValue`); the second grep prints the `Debug` impl line (`lib.rs:140`). If `rfc7616_md5_credentials_parse`, `rfc7616_sha256_credentials_parse`, `username_star_extended_parameter_parses` or `own_digest_challenge_parses_as_one_challenge` FAIL, take the E0 branch described above. If `basic_token68_credentials_are_not_parseable` FAILS (parser accepts it), Basic can also go through `ChallengeParser`: record that and skip the `headers` dependency in 10c, using `ParamValue`-less manual `Basic <b64>` handling via the `base64` crate instead.

- [ ] **Step 3: Record the verification outcome in `.delegate/w1-t-report.md`**

Add a section `## SP2.4 verification (Task 1)` with: the test output, the `grep -c` result, the three source references above, and the decision (parse = `ChallengeParser`; render = formatter + escaping, escalation E1 "no crate can render a WWW-Authenticate challenge: `http-auth::ChallengeRef` has Debug only"; Basic/Bearer = `headers`; Digest `username*`/`userhash` parameters are parsed but not honoured, same as before).

- [ ] **Step 4: Commit**

```bash
git add broadcast-auth/tests/http_auth_rfc7616.rs
git commit -m "test(broadcast-auth): verify http-auth ChallengeParser reads RFC 7616 credentials (SP2.4 gate)"
```

---

### Task 2: Goldens from main — broadcast-auth wire output

Spec §6: goldens are generated from main BEFORE the change and committed. Outputs: `SignedUrlKeySet::sign` query strings, `Verifier::challenge` for Basic/Bearer/Digest (nonce masked: it embeds a random per-verifier secret), and the client `Authorization` for Basic and Bearer.

**Files:**
- Create: `broadcast-auth/tests/golden_wire.rs`
- Create: `broadcast-auth/tests/golden/README.md`, `signed_url_sign.txt`, `challenges.txt`, `client_authorization.txt`

**Interfaces:** none (test-only).

- [ ] **Step 1: Write the golden test**

```rust
//! Byte-for-byte golden gate for broadcast-auth's wire output. The expected
//! files in `tests/golden/` were generated from unmodified `main` (commit in
//! `tests/golden/README.md`). `GOLDEN_BLESS=<dir>` writes instead of comparing.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use broadcast_auth::{Credentials, RequestContext, SignedUrlKeySet, Verifier, respond};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn keys() -> SignedUrlKeySet {
    SignedUrlKeySet::new([
        ("key-a".to_string(), b"01234567890123456789012345678901".to_vec()),
        ("kid_b.2-x".to_string(), b"abcdefghijabcdefghijabcdefghij01".to_vec()),
    ])
    .unwrap()
}

#[test]
fn signed_url_query_strings_match_golden() {
    let k = keys();
    let v4: IpAddr = "192.0.2.7".parse().unwrap();
    let v6: IpAddr = "2001:db8::1".parse().unwrap();
    let cases: [(&str, &str, u64, Option<IpAddr>); 5] = [
        ("key-a", "/live/stream.m3u8", 1_900_000_000, None),
        ("key-a", "/live/stream.m3u8", 1_900_000_000, Some(v4)),
        ("key-a", "/live/stream.m3u8", 1_900_000_000, Some(v6)),
        ("kid_b.2-x", "/vod/seg-1.m4s", 4_000_000_000, None),
        ("kid_b.2-x", "/vod/seg-1.m4s", 4_000_000_000, Some(v4)),
    ];
    let mut out = String::new();
    for (kid, path, exp, ip) in cases {
        let ip_text = ip.map_or_else(|| "-".to_string(), |i| i.to_string());
        let query = k.sign(kid, path, exp, ip).unwrap();
        out.push_str(&format!("{kid}\t{path}\t{exp}\t{ip_text}\t{query}\n"));
    }
    check("signed_url_sign.txt", &out);
}

/// The Digest nonce embeds a per-verifier random secret, so its hex is masked
/// to its length; everything else is byte-compared.
fn mask_nonce(challenge: &str) -> String {
    let start = challenge.find("nonce=\"").expect("digest nonce") + "nonce=\"".len();
    let len = challenge[start..].find('"').expect("closing quote");
    format!(
        "{}<nonce:{len} hex chars>{}",
        &challenge[..start],
        &challenge[start + len..]
    )
}

#[test]
fn challenges_match_golden() {
    let basic = Verifier::new(
        Credentials::Basic { username: "admin".into(), password: "12345".into() },
        "cameras",
    );
    let digest = Verifier::new(
        Credentials::Digest { username: "admin".into(), password: "12345".into() },
        "cameras",
    );
    let bearer = Verifier::new(Credentials::bearer("tok"), "cameras");
    let mut out = String::new();
    out.push_str(&format!("basic\t{}\n", basic.challenge()));
    out.push_str(&format!("digest\t{}\n", mask_nonce(&digest.challenge())));
    out.push_str(&format!("bearer\t{}\n", bearer.challenge()));
    check("challenges.txt", &out);
}

#[test]
fn client_authorization_matches_golden() {
    let ctx = RequestContext::new("GET", "/x");
    let basic = respond("Basic realm=\"cameras\"", &ctx, Credentials::new("admin", "12345")).unwrap();
    let bearer = respond("", &ctx, Credentials::bearer("mytoken123")).unwrap();
    check(
        "client_authorization.txt",
        &format!("basic\t{basic}\nbearer\t{bearer}\n"),
    );
}
```

(`Credentials::new` builds a `Digest` value — `credentials.rs:46-51` — so the Basic verifier is spelled out explicitly; the client calls use `Credentials::new` because the challenge, not the variant, picks the scheme.)

- [ ] **Step 2: Generate the goldens from UNMODIFIED code**

```bash
git status --short                      # must show only the new test file
GOLDEN_BLESS=$PWD/broadcast-auth/tests/golden cargo test -p broadcast-auth --locked --test golden_wire 2>&1 | grep -E 'test result'
ls broadcast-auth/tests/golden
cat broadcast-auth/tests/golden/*.txt
```

Expected: 3 tests pass (they only write); three `.txt` files; `client_authorization.txt` reads `basic\tBasic YWRtaW46MTIzNDU=` and `bearer\tBearer mytoken123`; `signed_url_sign.txt` has five lines, the IPv6 line ending `&ip=2001:db8::1`.

- [ ] **Step 3: README, then compare mode must PASS**

Write `broadcast-auth/tests/golden/README.md`: the baseline commit from Task 0, the exact bless command above, and "never regenerate on this branch; a deliberate difference is edited into the file in the commit that causes it and listed in CHANGELOG `[Unreleased]`."

```bash
cargo test -p broadcast-auth --locked --test golden_wire 2>&1 | grep -E 'test result|FAILED'
```

Expected: `test result: ok. 3 passed`.

- [ ] **Step 4: Commit**

```bash
git add broadcast-auth/tests/golden_wire.rs broadcast-auth/tests/golden
git commit -m "test(broadcast-auth): golden wire output (signed-URL query, challenges, client Authorization) from main"
```

---

### Task 3: Goldens from main — transmux RTP SDP and HLS IV

The MPD/Smooth/PlayReady goldens already exist (`transmux/tests/golden.rs`, generated from `b383d298`, they pass on main) and cover `cenc:pssh` base64, `default_KID` hex, Smooth `CodecPrivateData` hex and every `PT…S` duration string. This task adds the missing outputs: the RTP SDP (session from the `h264_aac.ts` fixture plus `build_sdp_with_connection` for IPv4 and IPv6) and the HLS `EXT-X-KEY` IV text.

**Files:**
- Create: `transmux/tests/golden_wire.rs`
- Create: `transmux/tests/golden/rtp-sdp-h264-aac.sdp`, `rtp-sdp-conn-v4.sdp`, `rtp-sdp-conn-v6.sdp`, `hls-iv.txt`
- Modify: `transmux/tests/golden/README.md` (append a section)

**Interfaces:** none (test-only). NOTE for Task 14: the connection-address cases call `build_sdp_with_connection(addr, "<str>")` (current signature); Task 14 changes the second parameter to `Vec<sdp_types::Media>` and updates this file in the same commit.

- [ ] **Step 1: Write the golden test**

```rust
//! Goldens for the RTP SDP and the HLS IV text, generated from unmodified
//! `main` (see `tests/golden/README.md`). `GOLDEN_BLESS=<dir>` writes.
#![cfg(feature = "std")]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use broadcast_common::{Package, Unpackage};
use transmux::{RtpPacketiser, TsDemux, build_sdp_with_connection};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

#[test]
fn rtp_session_sdp_matches_golden() {
    let ts = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"))
        .expect("ts fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let mut p = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise");
    check("rtp-sdp-h264-aac.sdp", &out.sdp);
}

const MEDIA_BLOCK: &str =
    "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1; profile-level-id=64000D; sprop-parameter-sets=Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA\r\n";

#[test]
fn connection_address_sdp_matches_golden() {
    let v4 = build_sdp_with_connection(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), MEDIA_BLOCK);
    let v6 = build_sdp_with_connection(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        "",
    );
    check("rtp-sdp-conn-v4.sdp", &v4);
    check("rtp-sdp-conn-v6.sdp", &v6);
}

#[cfg(feature = "sample-aes")]
#[test]
fn hls_iv_text_matches_golden() {
    let mut text = String::new();
    for iv in [
        [0u8; 16],
        [0xff; 16],
        [0x00, 0x01, 0x0a, 0x0b, 0xa0, 0xb0, 0xf0, 0x0f, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80],
    ] {
        text.push_str(&transmux::sample_aes::format_iv(&iv));
        text.push('\n');
    }
    check("hls-iv.txt", &text);
}
```

(`RtpPacketiser`, `TsDemux`, `build_sdp_with_connection` are crate-root re-exports — `lib.rs:337` for the last; `RtpPacketiser`'s `mtu`/`ssrc` fields are public, as `tests/rtp.rs:28-34` uses them.)

- [ ] **Step 2: Bless from unmodified code, inspect, compare**

```bash
GOLDEN_BLESS=$PWD/transmux/tests/golden cargo test -p transmux --all-features --locked --test golden_wire 2>&1 | grep -E 'test result'
cat transmux/tests/golden/rtp-sdp-h264-aac.sdp transmux/tests/golden/rtp-sdp-conn-v6.sdp transmux/tests/golden/hls-iv.txt
cargo test -p transmux --all-features --locked --test golden_wire 2>&1 | grep -E 'test result|FAILED'
```

Expected: first run 3 passed (writing); `rtp-sdp-h264-aac.sdp` starts `v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=transmux RTP\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=video 0 RTP/AVP 96…`; `hls-iv.txt` has `0x000000…` lines; second run `3 passed` in compare mode.

- [ ] **Step 3: Append to `transmux/tests/golden/README.md`**

Add a section "RTP SDP and HLS IV (W1-T)": the baseline commit, the bless command (`GOLDEN_BLESS=<dir> cargo test -p transmux --all-features --locked --test golden_wire`), and that Task 12 intentionally edits the `PT…S` goldens (listed in the CHANGELOG), the other files stay byte-identical.

- [ ] **Step 4: Commit**

```bash
git add transmux/tests/golden_wire.rs transmux/tests/golden/rtp-sdp-h264-aac.sdp transmux/tests/golden/rtp-sdp-conn-v4.sdp transmux/tests/golden/rtp-sdp-conn-v6.sdp transmux/tests/golden/hls-iv.txt transmux/tests/golden/README.md
git commit -m "test(transmux): golden RTP SDP and HLS IV text from main"
```

---

### Task 4: Goldens from main — timed-metadata DATERANGE/RFC 3339 and scte35-splice base64

**Files:**
- Create: `timed-metadata/tests/golden_wire.rs`, `timed-metadata/tests/golden/daterange.txt`, `timed-metadata/tests/golden/rfc3339.txt`, `timed-metadata/tests/golden/README.md`
- Create: `scte35-splice/tests/golden_base64.rs`, `scte35-splice/tests/golden/base64.txt`, `scte35-splice/tests/golden/README.md`

**Interfaces:** none (test-only). Public items used: `timed_metadata::daterange::{DateRange, Scte35Attr}` (check the exact path with `grep -n "pub mod\|pub use" timed-metadata/src/lib.rs`), `timed_metadata::anchor::format_rfc3339_ms`, `scte35_splice::dvb_ta::base64_encode`.

- [ ] **Step 1: timed-metadata golden test**

```rust
//! Goldens for DATERANGE rendering and RFC 3339 formatting, generated from
//! unmodified `main`. `GOLDEN_BLESS=<dir>` writes.
use std::fs;
use std::path::{Path, PathBuf};

use timed_metadata::DateRange;
use timed_metadata::anchor::format_rfc3339_ms;
use timed_metadata::daterange::{Scte35Attr, Scte35Cue};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn range(cue: Scte35Cue, raw: Vec<u8>) -> DateRange {
    DateRange {
        id: "2002".to_string(),
        start_date: "2018-10-29T10:38:00.000Z".to_string(),
        class: None,
        duration: None,
        planned_duration: Some(24.0),
        scte35: Some(Scte35Attr { cue, raw }),
        extra_attrs: Vec::new(),
    }
}

#[test]
fn daterange_tag_lines_match_golden() {
    let long: Vec<u8> = (0u8..17).map(|i| i.wrapping_mul(0x0F)).collect();
    let cases = [
        range(Scte35Cue::Out, vec![0xFC, 0x30, 0x21]),
        range(Scte35Cue::In, vec![0x00]),
        range(Scte35Cue::Cmd, vec![0x0A, 0xA0, 0x0F, 0xFF, 0x01, 0xBE]),
        range(Scte35Cue::Out, long),
    ];
    let mut out = String::new();
    for dr in cases {
        out.push_str(&dr.to_tag_line().unwrap());
        out.push('\n');
    }
    check("daterange.txt", &out);
}

#[test]
fn rfc3339_formatting_matches_golden() {
    let epoch_ms: [i64; 12] = [
        0,
        1,
        999,
        1_000,
        86_400_000,
        -1,
        -86_400_000,
        951_782_400_000,      // 2000-02-29T00:00:00Z (leap day)
        4_107_542_400_000,    // 2100-03-01T00:00:00Z (2100 is not a leap year)
        1_700_000_000_123,
        253_402_300_799_999,  // 9999-12-31T23:59:59.999Z
        -62_135_596_800_000,  // 0001-01-01T00:00:00Z
    ];
    let mut out = String::new();
    for ms in epoch_ms {
        out.push_str(&format!("{ms}\t{}\n", format_rfc3339_ms(ms)));
    }
    check("rfc3339.txt", &out);
}
```

(`DateRange` is re-exported at `timed-metadata/src/lib.rs:37`; `Scte35Attr`/`Scte35Cue` live in `daterange` — fields read from `daterange.rs:48-82`; `extra_attrs` is the real field name.)

- [ ] **Step 2: scte35-splice golden test**

```rust
//! Golden for `dvb_ta::base64_encode`, generated from unmodified `main`.
use std::{fs, path::{Path, PathBuf}};
use scte35_splice::dvb_ta::base64_encode;

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

#[test]
fn base64_encode_matches_golden() {
    // RFC 4648 §10 test vectors plus every length mod 3 of a binary blob.
    let vectors: [&[u8]; 9] = [b"", b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar",
        &[0xFC, 0x30, 0x0A, 0x00, 0xFF, 0xFE, 0xFB], &[0xFB, 0xEF, 0xBE, 0x3F, 0xFF]];
    let mut out = String::new();
    for v in vectors {
        out.push_str(&format!("{}\t{}\n", v.len(), String::from_utf8(base64_encode(v)).unwrap()));
    }
    check("base64.txt", &out);
}
```

- [ ] **Step 3: Bless from unmodified code, inspect, compare**

```bash
GOLDEN_BLESS=$PWD/timed-metadata/tests/golden cargo test -p timed-metadata --all-features --locked --test golden_wire 2>&1 | grep -E 'test result'
GOLDEN_BLESS=$PWD/scte35-splice/tests/golden cargo test -p scte35-splice --all-features --locked --test golden_base64 2>&1 | grep -E 'test result'
cat timed-metadata/tests/golden/rfc3339.txt scte35-splice/tests/golden/base64.txt
cargo test --locked --all-features -p timed-metadata --test golden_wire -p scte35-splice --test golden_base64 2>&1 | grep -E 'test result|FAILED'
```

Expected: `rfc3339.txt` first line `0\t1970-01-01T00:00:00.000Z`, the `-1` line `1969-12-31T23:59:59.999Z`, the 2100 line `2100-03-01T00:00:00.000Z`; `base64.txt` lines `0\t` (empty), `1\tZg==`, `6\tZm9vYmFy`. Compare run passes.

- [ ] **Step 4: READMEs (baseline commit + bless command) and commit**

```bash
git add timed-metadata/tests/golden_wire.rs timed-metadata/tests/golden scte35-splice/tests/golden_base64.rs scte35-splice/tests/golden
git commit -m "test(timed-metadata,scte35-splice): golden DATERANGE, RFC 3339 and base64 output from main"
```

---

### Task 5: broadcast-common — `hex` delegates to the `hex` crate (not breaking)

**Files:**
- Modify: `broadcast-common/Cargo.toml` (`[dependencies]`, after `libm`)
- Modify: `broadcast-common/src/hex.rs:11-35` (module doc + `hex_encode` body)
- Create: `broadcast-common/tests/hex_parity.rs`

**Interfaces:** `broadcast_common::hex::hex_encode(&[u8]) -> String` — signature and output unchanged (lowercase, two chars per byte, no prefix). New dependency edge: `hex` 0.4.3 `default-features = false, features = ["alloc"]` (no_std-capable; verified in `hex-0.4.3/src/lib.rs`: `#[cfg(feature = "alloc")] extern crate alloc`).

- [ ] **Step 1: Pin test (characterisation: passes on the old and the new body)**

```rust
//! `hex_encode` is the public API the hex crate now backs: pin its exact
//! contract so the delegation cannot change it.
use broadcast_common::hex::hex_encode;

#[test]
fn every_byte_value_is_two_lowercase_digits() {
    for b in 0..=255u8 {
        assert_eq!(hex_encode(&[b]), format!("{b:02x}"));
    }
}

#[test]
fn long_input_has_no_separators_or_prefix() {
    let data: Vec<u8> = (0..=255u8).cycle().take(1021).collect();
    let text = hex_encode(&data);
    assert_eq!(text.len(), 2042);
    assert!(text.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    assert_eq!(hex_encode(&[]), "");
}
```

- [ ] **Step 2: Run on the OLD body — PASS (pins current behaviour)**

```bash
cargo test -p broadcast-common --all-features --locked --test hex_parity 2>&1 | grep -E 'test result|FAILED'
```

Expected: `2 passed`.

- [ ] **Step 3: Implement**

`Cargo.toml`:
```toml
# Hex codec backing `hex::hex_encode` (SP5). `alloc` only: the crate stays no_std.
hex = { version = "0.4", default-features = false, features = ["alloc"] }
```
Follow the dependency-add procedure. In `hex.rs` replace the body of `hex_encode` and trim the module doc's "Only the encoder is shared" paragraph to say the encoder delegates to the `hex` crate and decoders stay with their callers' error types:
```rust
pub fn hex_encode(data: &[u8]) -> String {
    ::hex::encode(data)
}
```
(`::hex` is the extern crate; the local module is also named `hex`, so the leading `::` is required.) Delete the now-unused `use alloc::string::String` only if the compiler says so (the return type still needs `String` in scope — keep it).

- [ ] **Step 4: Run — PASS, all workspace dependents still build, no_std build**

```bash
cargo test -p broadcast-common --all-features --locked 2>&1 | grep -E 'test result|FAILED'
cargo build -p broadcast-common --no-default-features --locked 2>&1 | tail -2
cargo check --workspace --all-features --locked 2>&1 | grep -E '^error' | head
```

Expected: all `ok`; `Finished`; no output from the last command. Also `cargo tree -p broadcast-common -e normal --locked | grep -E 'hex'` shows `hex v0.4.3`.

- [ ] **Step 5: Commit**

```bash
git add broadcast-common/Cargo.toml broadcast-common/src/hex.rs broadcast-common/tests/hex_parity.rs Cargo.lock
git commit -m "refactor(broadcast-common): hex_encode delegates to the hex crate (API unchanged)"
```

---

### Task 6a: transmux — `hex` crate at every hex site (fixes the CLI `--key` panic)

Sites (current lines): `rtp.rs:2149-2197` (`hex_decode`), `smooth_parse.rs:596-634` (`hex_decode` + `hex_nibble`), `smooth.rs:853-861` (`hex_upper`), `dash.rs:1232-1255` (`format_kid` + `hex_lower`), `sample_aes.rs:573-584` (`format_iv`), `cli.rs:1321-1334` (`parse_hex16`). Real defect found: `parse_hex16` slices `&s[i*2..i*2+2]` after only checking `s.len() == 32` (BYTES), so a non-ASCII `--key` argument such as `"a" + "é"×15 + "b"` panics on a char boundary, and `u8::from_str_radix` accepts a leading `+` (`"+1"` parses as `1`), so `--key` accepts malformed hex.

**Files:**
- Modify: `transmux/Cargo.toml` (`[dependencies]`: add `hex = { version = "0.4", default-features = false, features = ["alloc"] }`)
- Modify: `transmux/src/rtp.rs:2149-2197`, `transmux/src/smooth_parse.rs:596-634`, `transmux/src/smooth.rs:853-861` (+ test at `:962-965`), `transmux/src/dash.rs:1232-1255`, `transmux/src/sample_aes.rs:573-584`, `transmux/src/cli.rs:1321-1334`
- Test: `transmux/src/cli.rs` (new `#[cfg(all(test, feature = "cenc"))] mod parse_hex16_tests` next to `key_redaction_tests`)

**Interfaces (all unchanged):** `transmux::rtp::hex_decode(&str) -> Result<Vec<u8>>` keeps its two error reasons (`"odd-length hex string"`, `"not a hex digit"`); `transmux::smooth_parse::hex_decode` keeps `SmoothParseError::{CodecPrivateDataTooLong, InvalidHex}` and its length bound; `sample_aes::format_iv(&[u8; 16]) -> String` (`0x` + 32 lowercase hex).

- [ ] **Step 1: Write the failing regression tests (defect: panic + `+` accepted)**

```rust
#[cfg(all(test, feature = "cenc"))]
mod parse_hex16_tests {
    use super::parse_hex16;

    /// 32 BYTES but not 32 hex chars: `"a" + 15 × "é" + "b"`. The old
    /// `&s[i * 2..i * 2 + 2]` slice cuts the first `é` in half and panics.
    #[test]
    fn non_ascii_input_of_the_right_byte_length_is_none_not_a_panic() {
        let s = format!("a{}b", "é".repeat(15));
        assert_eq!(s.len(), 32);
        assert_eq!(parse_hex16(&s), None);
    }

    /// `u8::from_str_radix("+1", 16)` is `Ok(1)`, so the old parser accepted a
    /// sign in place of a hex digit.
    #[test]
    fn a_sign_is_not_a_hex_digit() {
        assert_eq!(parse_hex16(&"+1".repeat(16)), None);
        assert_eq!(parse_hex16(&"-1".repeat(16)), None);
    }

    #[test]
    fn valid_key_still_parses_and_whitespace_is_trimmed() {
        let hex = "0102030405060708090a0b0c0d0e0f10";
        assert_eq!(
            parse_hex16(&format!("  {hex}\n")),
            Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
        assert_eq!(parse_hex16("0102"), None);
    }
}
```

- [ ] **Step 2: Run — FAIL (revert-check evidence for the defect is this very run)**

```bash
cargo test -p transmux --all-features --locked --lib parse_hex16_tests 2>&1 | grep -E 'test |panicked|test result'
```

Expected: `non_ascii_input_of_the_right_byte_length_is_none_not_a_panic` FAILS with `panicked at … byte index 1 is not a char boundary`; `a_sign_is_not_a_hex_digit` FAILS (`left: Some([1, 1, …]) right: None`); the third passes. Paste this into the report under "Revert-check evidence → Task 6a" (the failing run on the old code IS the evidence).

- [ ] **Step 3: Implement**

Add the dependency (dependency-add procedure). Replace:

```rust
// cli.rs
#[cfg(feature = "cenc")]
fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    hex::decode_to_slice(s.trim(), &mut out).ok()?;
    Some(out)
}
```
(`hex::decode_to_slice` requires exactly 32 digits for 16 bytes, rejects every non-hex byte including `+`/`-`/non-ASCII, and never slices the `&str`. Delete the old body and its "Parse exactly 32 hex chars" comment's implementation detail.)

```rust
// rtp.rs
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    ::hex::decode(s).map_err(|e| match e {
        ::hex::FromHexError::OddLength => Error::InvalidValue {
            field: "hex",
            value: s.len() as u64,
            reason: "odd-length hex string",
        },
        ::hex::FromHexError::InvalidHexCharacter { c, .. } => Error::InvalidValue {
            field: "hex",
            value: u64::from(c),
            reason: "not a hex digit",
        },
        ::hex::FromHexError::InvalidStringLength => Error::InvalidValue {
            field: "hex",
            value: s.len() as u64,
            reason: "invalid hex string length",
        },
    })
}
```
(`::hex` because `rtp.rs` has `use broadcast_common::hex::hex_encode` and a sibling item named `hex` could shadow; `FromHexError` has exactly these three variants in 0.4.3 — `hex-0.4.3/src/error.rs`.)

```rust
// smooth_parse.rs — keep the length bound BEFORE decoding
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() > MAX_CODEC_PRIVATE_DATA_HEX_LEN {
        return Err(SmoothParseError::CodecPrivateDataTooLong { len: s.len() });
    }
    ::hex::decode(s).map_err(|_| SmoothParseError::InvalidHex { value: s.to_string() })
}
```
and delete `hex_nibble`. `hex::decode("")` is `Ok(vec![])`, matching the old empty-string branch.

```rust
// smooth.rs: delete `hex_upper`; at the three call sites (lines ~496, 499, 510)
ql.push(("CodecPrivateData", hex::encode_upper(&s.codec_private_data)));
// and replace the unit test `hex_upper_encodes` (line ~962) with an assertion on
// hex::encode_upper(&[0x00, 0x01, 0xAB, 0xFF]) == "0001ABFF" -- or delete it: the
// golden smooth.manifest.xml covers the writer end to end.
```
```rust
// dash.rs
fn format_kid(kid: &[u8; 16]) -> String {
    let hex = hex::encode(kid);
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}
// delete hex_lower
```
```rust
// sample_aes.rs
pub fn format_iv(iv: &[u8; BLOCK_LEN]) -> String {
    format!("0x{}", hex::encode(iv))
}
```

- [ ] **Step 4: Run — PASS, goldens untouched**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p transmux --no-default-features --locked 2>&1 | tail -2
cargo check -p multimux --all-features --locked 2>&1 | tail -1
```

Expected: every suite `ok` (including `golden`, `golden_wire`, `smooth_parse` hex tests, `rtp`); `Finished`; multimux `Finished`.

- [ ] **Step 5: Revert-check**

After committing, edit `parse_hex16` back to the old loop (`for (i, byte) in out.iter_mut().enumerate() { *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?; }` with the `if s.len() != 32 { return None; }` guard), run `cargo test -p transmux --all-features --locked --lib parse_hex16_tests`, expect the two defect tests FAIL, record, then `git checkout -- transmux/src/cli.rs` and re-run PASS.

- [ ] **Step 6: Commit**

```bash
git add transmux/Cargo.toml Cargo.lock transmux/src/rtp.rs transmux/src/smooth_parse.rs transmux/src/smooth.rs transmux/src/dash.rs transmux/src/sample_aes.rs transmux/src/cli.rs
git commit -m "refactor(transmux): hex crate at every hex site; fix --key parse panic on non-ASCII and sign-accepting hex"
```

---

### Task 6b: timed-metadata — `hex` crate for DATERANGE `SCTE35-*` (fixes a panic)

Same defect class: `daterange.rs:282-297` `parse_hex` slices `&h[i..i + 2]` on a `&str` (panics on a multi-byte char: `0xaéb`) and `u8::from_str_radix` accepts `+` (`0x+1+1` ⇒ `[1, 1]`). Reachable from any DATERANGE playlist line a remote origin serves.

**Files:**
- Modify: `timed-metadata/Cargo.toml` (`[dependencies]`: `hex = { version = "0.4", default-features = false, features = ["alloc"] }`)
- Modify: `timed-metadata/src/daterange.rs:127-132` (render), `:185-195` (parse), `:272-297` (delete `to_hex_upper`, rewrite `parse_hex`), `:300-306` (the `hex_upper_is_zero_padded_uppercase` unit test of the deleted fn)
- Test: `timed-metadata/tests/daterange_hex.rs` (new, public API only so the revert-check can restore `src/`)

**Interfaces:** `DateRange::{to_tag_line, parse_tag_line}` unchanged; `Error::AttrParse` messages `"odd-length hex"` / `"bad hex"` kept.

- [ ] **Step 1: Failing regression tests**

```rust
//! `SCTE35-*` attribute hex is attacker-reachable (it comes from a remote
//! playlist): it must never panic and must be strict hex.
use timed_metadata::DateRange;

fn line(value: &str) -> String {
    format!("#EXT-X-DATERANGE:ID=\"x\",START-DATE=\"2020-01-01T00:00:00Z\",SCTE35-OUT={value}")
}

#[test]
fn multibyte_character_in_hex_is_an_error_not_a_panic() {
    // 4 bytes, even length: the old `&h[0..2]` cuts the `é` in half.
    assert!(DateRange::parse_tag_line(&line("0xaéb")).is_err());
}

#[test]
fn a_sign_is_not_a_hex_digit() {
    // old: from_str_radix("+1", 16) == Ok(1) => bytes [1, 1]
    assert!(DateRange::parse_tag_line(&line("0x+1+1")).is_err());
    assert!(DateRange::parse_tag_line(&line("0x-1-1")).is_err());
}

#[test]
fn odd_length_and_non_hex_are_errors() {
    assert!(DateRange::parse_tag_line(&line("0xABC")).is_err());
    assert!(DateRange::parse_tag_line(&line("0xZZ")).is_err());
}

#[test]
fn valid_hex_round_trips_uppercase_and_accepts_either_prefix_case() {
    let dr = DateRange::parse_tag_line(&line("0xFC3021")).unwrap();
    assert_eq!(dr.scte35.as_ref().unwrap().raw, vec![0xFC, 0x30, 0x21]);
    assert!(dr.to_tag_line().unwrap().contains("SCTE35-OUT=0xFC3021"));
    let lower = DateRange::parse_tag_line(&line("0Xfc3021")).unwrap();
    assert_eq!(lower.scte35.unwrap().raw, vec![0xFC, 0x30, 0x21]);
}
```

- [ ] **Step 2: Run — FAIL**

```bash
cargo test -p timed-metadata --all-features --locked --test daterange_hex 2>&1 | grep -E 'test |panicked|test result'
```

Expected: `multibyte_character…` FAILS (`panicked … is not a char boundary`), `a_sign_is_not_a_hex_digit` FAILS (`parse_tag_line` returned `Ok`), the other two pass. Record in the report.

- [ ] **Step 3: Implement**

At the render site (`daterange.rs:131`): `AttrValue::bare(format!("0x{}", hex::encode_upper(&s.raw)))?`. Replace `parse_hex`:
```rust
fn parse_hex(v: &str) -> Result<Vec<u8>> {
    let digits = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    hex::decode(digits).map_err(|e| match e {
        hex::FromHexError::OddLength => Error::AttrParse("odd-length hex".to_string()),
        _ => Error::AttrParse("bad hex".to_string()),
    })
}
```
Delete `to_hex_upper` and its unit test `hex_upper_is_zero_padded_uppercase` (the function no longer exists; the golden `daterange.txt` from Task 4 and `tag_round_trips_byte_identical` cover the rendering — record this deletion in the report, it is not a weakening).

- [ ] **Step 4: Run — PASS, golden byte-identical**

```bash
cargo test -p timed-metadata --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p timed-metadata --no-default-features --locked 2>&1 | tail -2
```

Expected: all `ok` including `golden_wire` (DATERANGE hex is uppercase, zero-padded) and `daterange_fixture`.

- [ ] **Step 5: Revert-check, then commit**

Restore the old `parse_hex` body in `src/daterange.rs` (keep the hex import unused-allowed with `#[allow(unused)]` for the test run), run `--test daterange_hex`, expect the two defect tests FAIL, `git checkout -- timed-metadata/src/daterange.rs`, re-run PASS.

```bash
git add timed-metadata/Cargo.toml Cargo.lock timed-metadata/src/daterange.rs timed-metadata/tests/daterange_hex.rs
git commit -m "fix(timed-metadata): DATERANGE SCTE35 hex via the hex crate; no panic on multibyte input, no sign digits"
```

---

### Task 7: base64 crate in transmux and scte35-splice (keeps decode leniency)

Review Focus #2. Sites: transmux `rtp.rs:2088-2147` (alphabet const, `base64_encode`, `base64_decode`; consumed by `rtp_sdp.rs:138` for `sprop-parameter-sets`, `drm.rs:145,355`, `dash.rs:1227`, multimux `output/whep.rs:344,350`), scte35-splice `dvb_ta/stream_event.rs:359-387` (`base64_encode -> Vec<u8>`).

**Files:**
- Modify: `transmux/Cargo.toml` (add `base64 = { version = "0.23", default-features = false, features = ["alloc"] }`), `transmux/src/rtp.rs:2088-2147` and its section banner
- Modify: `scte35-splice/Cargo.toml` (move `base64` from `[dev-dependencies]` to `[dependencies]` as `base64 = { version = "0.23", default-features = false, features = ["alloc"] }`, keep the dev line only if tests use `std` features — they use `base64::Engine` so delete the dev entry), `scte35-splice/src/dvb_ta/stream_event.rs:359-387` (+ unit tests at `:495-503` stay)
- Test: `transmux/tests/base64_lenient.rs` (new), `scte35-splice/tests/golden_base64.rs` (Task 4, must stay green)

**Interfaces (unchanged signatures):**
- `transmux::rtp::base64_encode(&[u8]) -> String` — standard alphabet, `=` padded.
- `transmux::rtp::base64_decode(&str) -> Result<Vec<u8>>` — padding optional, non-zero trailing bits accepted (documented leniency); invalid byte → `Error::InvalidValue { field: "base64", reason: "not a base64 character", .. }`.
- `scte35_splice::dvb_ta::base64_encode(&[u8]) -> Vec<u8>`.

- [ ] **Step 1: Failing/pin tests (transmux)**

```rust
//! RFC 4648 §10 vectors, plus the leniency real SDP producers depend on.
use transmux::rtp::{base64_decode, base64_encode};
use transmux::rtp_sdp::avc_config_from_sprop;
use broadcast_common::Serialize;

const VECTORS: [(&str, &str); 7] = [
    ("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"),
    ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy"),
];

#[test]
fn rfc4648_section_10_vectors_encode_and_decode() {
    for (plain, b64) in VECTORS {
        assert_eq!(base64_encode(plain.as_bytes()), b64);
        assert_eq!(base64_decode(b64).unwrap(), plain.as_bytes());
    }
}

#[test]
fn unpadded_input_decodes_like_padded() {
    for (plain, b64) in VECTORS {
        assert_eq!(base64_decode(b64.trim_end_matches('=')).unwrap(), plain.as_bytes());
    }
}

/// `Zh==` carries non-zero trailing bits; the old decoder ignored them and
/// strict base64 rejects them. Real encoders occasionally emit them.
#[test]
fn non_zero_trailing_bits_are_tolerated() {
    assert_eq!(base64_decode("Zh==").unwrap(), b"f");
}

#[test]
fn invalid_bytes_and_whitespace_are_errors() {
    for bad in ["Zm9v!", "Zm 9v", "Zm9v\n", "Zm9v=YmFy", "-_-_"] {
        assert!(base64_decode(bad).is_err(), "{bad:?}");
    }
}

/// The real ffmpeg High-profile `sprop-parameter-sets` (fixture
/// `tests/fixtures/rtp/high-ffmpeg.sdp`) with its padding removed must give a
/// byte-identical avcC.
#[test]
fn unpadded_real_sprop_gives_identical_avcc() {
    let padded = "Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA";
    let unpadded = "Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA,aOvjyyLA";
    let ser = |s: &str| {
        let cfg = avc_config_from_sprop(s).expect("sprop");
        let mut out = vec![0u8; cfg.config.serialized_len()];
        cfg.config.serialize_into(&mut out).unwrap();
        out
    };
    assert_eq!(ser(unpadded), ser(padded));
}
```

(`avc_config_from_sprop` is `rtp_sdp.rs:120`, returns `Result<AVCConfigurationBox>`; `.config.serialized_len()` follows the exact usage at `tests/rtp.rs:604-612`.)

- [ ] **Step 2: Run on the OLD code — PASS (pins the leniency the shim must keep)**

```bash
cargo test -p transmux --all-features --locked --test base64_lenient 2>&1 | grep -E 'test |test result'
```

Expected: 5 passed. If `invalid_bytes_and_whitespace_are_errors` FAILS on old code because a listed string is accepted (`Zm9v=YmFy`, the old decoder stripped every `=`), remove ONLY that string from the list and record it as an intentional tightening in the report/CHANGELOG ("stray `=` inside the data is now an error").

- [ ] **Step 3: Implement (transmux)**

Replace `rtp.rs:2088-2147` (banner `Hand-rolled base64…` → `base64 (RFC 4648) via the base64 crate`):
```rust
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Decoder used for SDP `sprop-parameter-sets`, DRM headers and Smooth/DASH
/// payloads: padding optional, trailing bits tolerated. Real producers emit
/// unpadded and non-canonical base64; the crate default is strict.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Base64-encode bytes (RFC 4648 §4, with `=` padding).
pub fn base64_encode(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Base64-decode a string (RFC 4648 §4); padding is optional and non-canonical
/// trailing bits are tolerated; any other invalid input is an error.
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    LENIENT.decode(s).map_err(|e| match e {
        base64::DecodeError::InvalidByte(_, byte) => Error::InvalidValue {
            field: "base64",
            value: u64::from(byte),
            reason: "not a base64 character",
        },
        _ => Error::InvalidValue {
            field: "base64",
            value: s.len() as u64,
            reason: "invalid base64 length",
        },
    })
}
```
The old `base64_round_trip`/`base64_known_vector` unit tests (`rtp.rs:2543-2556`) stay green unchanged.

scte35-splice (`stream_event.rs`): replace the alphabet const + loop:
```rust
#[must_use]
pub fn base64_encode(data: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data).into_bytes()
}
```
Update the doc sentence "Provided as a `no_std`/`alloc` convenience so callers do not pull a base-64 dependency" to "Thin wrapper over the `base64` crate (standard alphabet, `=` padding)". Follow the dependency-add procedure for both manifests.

- [ ] **Step 4: Run — PASS everywhere that touches base64**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo test -p scte35-splice --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p transmux -p scte35-splice --no-default-features --locked 2>&1 | tail -2
cargo check -p multimux --all-features --locked 2>&1 | tail -1
```

Expected: all `ok` (including `drm_pssh`, `dash_mpd` pssh assertions, `golden` protected MPD, `rtp`, `dvb_ta`, `golden_base64`); builds `Finished`.

- [ ] **Step 5: Commit**

```bash
git add transmux/Cargo.toml scte35-splice/Cargo.toml Cargo.lock transmux/src/rtp.rs scte35-splice/src/dvb_ta/stream_event.rs transmux/tests/base64_lenient.rs
git commit -m "refactor: base64 crate replaces the hand-rolled codecs in transmux and scte35-splice"
```

---

### Task 8: broadcast-auth — `lru` nonce table and `hex` nonce codec

Sites: `server.rs:96` (`BTreeMap` import), `:198-300` (`DigestNonces`, `NcEntry.last_used`, `NcTable.lru`/`next_use`), `:366-412` (`record`, `below_floor`), `:885-905` (`hex`/`unhex`). The semantics are security-relevant (anti-replay floor), so the refactor preserves them exactly and adds a pin test for the one property `lru` now owns (use order).

**Files:**
- Modify: `broadcast-auth/Cargo.toml` (`lru = "0.18"`, `hex = { version = "0.4", default-features = false, features = ["alloc"] }`)
- Modify: `broadcast-auth/src/server.rs` (ranges above)

**Interfaces:** no public change. Internal: `NcTable { entries: LruCache<PairKey, NcEntry>, floor_seq, next_seq, latest_issue_time, capacity }` (the `capacity` field stays: eviction is driven by our own loop so `set_capacity` shrinking still raises `floor_seq` for every live pair it drops, exactly like today — `LruCache::resize` would drop silently).

- [ ] **Step 1: Pin tests in `server.rs`'s `mod tests` (pass before AND after)**

```rust
    /// Eviction order is least-recently-USED, not least-recently-inserted:
    /// touching an old pair must protect it.
    #[test]
    fn recently_used_pair_survives_eviction() {
        let nonces = DigestNonces::new();
        nonces.set_capacity(2);
        let stamp = |seq| NonceStamp { time: 100, seq };
        let (a, b, c) = (pair_key("n1", "c"), pair_key("n2", "c"), pair_key("n3", "c"));
        assert!(nonces.record(a, stamp(1), 1, 100));
        assert!(nonces.record(b, stamp(2), 1, 100));
        // Use `a` again (nc 2), making `b` the least recently used.
        assert!(nonces.record(a, stamp(1), 2, 100));
        // Inserting `c` evicts `b`, not `a`.
        assert!(nonces.record(c, stamp(3), 1, 100));
        assert!(!nonces.record(a, stamp(1), 2, 100), "a still tracked: nc 2 is a replay");
        assert_eq!(nonces.table().entries.len(), 2);
        // `b` was evicted while still live, so its issue sequence is now below
        // the anti-replay floor and the pair cannot be re-admitted.
        assert!(!nonces.record(b, stamp(2), 1, 100));
    }

    #[test]
    fn nonce_is_96_lowercase_hex_and_round_trips() {
        let nonces = DigestNonces::new();
        let nonce = nonces.issue(1_000);
        assert_eq!(nonce.len(), NONCE_LEN * 2);
        assert!(nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        let stamp = nonces.issued_at(&nonce).expect("our own nonce verifies");
        assert_eq!(stamp.time, 1_000);
        assert!(nonces.issued_at(&nonce.to_uppercase()).is_some(), "hex is case-insensitive on read");
        assert!(nonces.issued_at(&nonce[..nonce.len() - 2]).is_none());
        assert!(nonces.issued_at(&format!("{}zz", &nonce[..nonce.len() - 2])).is_none());
    }
```
(`NonceStamp`'s fields are private to the module — the test module is a child, so it can build one.)

- [ ] **Step 2: Run on OLD code — PASS**

```bash
cargo test -p broadcast-auth --all-features --locked recently_used_pair nonce_is_96 2>&1 | grep -E 'test |test result'
```

Expected: 2 passed. (A pin on the old implementation is the point; if `recently_used_pair_survives_eviction` fails on old code, the test misreads the semantics — fix the TEST, not the code.)

- [ ] **Step 3: Implement**

```rust
use lru::LruCache;
use std::collections::HashMap;   // keep only if still used; remove BTreeMap

struct NcEntry {
    stamp: NonceStamp,
    highest: u32,
    window: u64,
    // `last_used` removed: LruCache owns the use order.
}

struct NcTable {
    /// Per-`(nonce, cnonce)` anti-replay state, in least-recently-used order.
    /// Unbounded as far as `lru` is concerned: the cap below is enforced by
    /// `record` so a dropped live pair can raise `floor_seq`.
    entries: LruCache<PairKey, NcEntry>,
    capacity: usize,
    floor_seq: u64,
    next_seq: u64,
    latest_issue_time: u64,
}
```
`DigestNonces::new()`: `entries: LruCache::unbounded()` (lazy allocation — `LruCache::new(cap)` would pre-allocate 65 536 slots per verifier). `record`:
```rust
    fn record(&self, key: PairKey, stamp: NonceStamp, nc: u32, now_secs: u64) -> bool {
        let mut guard = self.table();
        let table = &mut *guard;
        if let Some(entry) = table.entries.get_mut(&key) {   // get_mut promotes to most recent
            return entry.accept(nc);
        }
        while table.entries.len() >= table.capacity {
            let Some((_, dropped)) = table.entries.pop_lru() else { break };
            if !Self::is_expired(dropped.stamp.time, now_secs) {
                table.floor_seq = table.floor_seq.max(dropped.stamp.seq.saturating_add(1));
            }
        }
        if stamp.seq < table.floor_seq {
            return false;
        }
        table.entries.put(key, NcEntry { stamp, highest: nc, window: 0 });
        true
    }
```
The old code touched the use order only after a SUCCESSFUL `accept` (a rejected replay returned early), whereas `get_mut` would promote on a rejected replay too. Preserve that exactly with `peek_mut` for the accept test and `promote` on success:
```rust
        if let Some(entry) = table.entries.peek_mut(&key) {
            if !entry.accept(nc) { return false; }
            table.entries.promote(&key);
            return true;
        }
```
(`LruCache::peek_mut` and `promote` both exist in lru 0.18.5 — `lib.rs:1176` and `:1439`.) `below_floor`: `!table.entries.contains(key)`. `set_capacity` unchanged (`self.table().capacity = capacity.max(1)`). Delete `next_use`, `last_used`, the `BTreeMap` import. Hex:
```rust
        hex::encode(raw)            // in `issue`
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    hex::decode_to_slice(text, &mut out).ok()?;   // exact length, hex digits only
    Some(out)
}
```
Delete `fn hex`. Follow the dependency-add procedure.

- [ ] **Step 4: Run — PASS, goldens identical**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected: all `ok` (existing nc-window/eviction/stale tests at `server.rs:1700-1900`, `golden_wire`, `http_auth_rfc7616`, the two new pins).

- [ ] **Step 5: Commit**

```bash
git add broadcast-auth/Cargo.toml Cargo.lock broadcast-auth/src/server.rs
git commit -m "refactor(broadcast-auth): lru crate for the Digest nc table, hex crate for nonces"
```

---

### Task 9: broadcast-auth — signed URLs through `form_urlencoded` (defect 6)

Defect 6: `sign` builds `format!("exp={exp}&kid={kid}&sig={sig}")` with `kid` raw, and `verify` parses with `split('&')`/`split_once('=')` and never decodes. A `kid` containing `& = % +` or a space therefore cannot round-trip (it truncates, smuggles parameters, or is mis-read).

**Files:**
- Modify: `broadcast-auth/Cargo.toml` (`form_urlencoded = "1"`)
- Modify: `broadcast-auth/src/signed_url.rs:145-157` (`sign`), `:196-216` (`split_path_and_query` stays, `query_get` replaced), `:240-290` (`verify`), `:550-556` (test `query_get_skips_malformed_pairs_without_panicking`), module docs `:6-30` (wire form: percent-encoded, `application/x-www-form-urlencoded`)
- Modify: `broadcast-auth/tests/golden/signed_url_sign.txt` (the IPv6 line only)
- Test: `broadcast-auth/tests/signed_url_kid.rs` (new, public API only)

**Interfaces:**
- `SignedUrlKeySet::sign(&self, kid, path, exp, ip) -> Result<String>` — same signature; the returned query is now `application/x-www-form-urlencoded`-encoded (`form_urlencoded::Serializer`), parameter order unchanged (`exp`, `kid`, `sig`, `ip`).
- `Verifier::verify` for `SignedUrl` — percent-decodes every value (`form_urlencoded::parse`), first occurrence of a key wins, a bare key without `=` is `""`.
- Wire difference (CHANGELOG, breaking): a `kid` is percent-encoded; `ip=2001:db8::1` becomes `ip=2001%3Adb8%3A%3A1`; `+` in a `kid` now reads as a space.

- [ ] **Step 1: Failing regression tests (defect 6)**

```rust
//! Defect 6 (spec §3): the signed-URL `kid` was not percent-encoded.
use std::net::{IpAddr, SocketAddr};
use broadcast_auth::{AuthResult, RequestContext, SignedUrlKeySet, Verifier};

const SECRET: &[u8; 32] = b"01234567890123456789012345678901";
const NASTY_KID: &str = "team a/b&c=d%e+f";

fn keys(kid: &str) -> SignedUrlKeySet {
    SignedUrlKeySet::new([(kid.to_string(), SECRET.to_vec())]).unwrap()
}
fn far_future() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600
}
fn verify(verifier: &Verifier, uri: &str, peer: Option<SocketAddr>) -> AuthResult {
    let mut ctx = RequestContext::new("GET", uri);
    if let Some(p) = peer { ctx = ctx.with_peer_addr(p); }
    verifier.verify(&ctx)
}

#[test]
fn kid_with_reserved_characters_round_trips() {
    let query = keys(NASTY_KID).sign(NASTY_KID, "/s/m.m3u8", far_future(), None).unwrap();
    // Exactly three parameters: the kid cannot smuggle extra ones.
    let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    assert_eq!(pairs.len(), 3, "{query}");
    assert_eq!(pairs[1], ("kid".to_string(), NASTY_KID.to_string()));
    let verifier = Verifier::signed_url(keys(NASTY_KID));
    assert_eq!(verify(&verifier, &format!("/s/m.m3u8?{query}"), None), AuthResult::Ok);
}

#[test]
fn kid_cannot_inject_an_ip_binding_or_expiry() {
    let kid = "k&ip=198.51.100.9&exp=1";
    let query = keys(kid).sign(kid, "/p", far_future(), None).unwrap();
    let pairs: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
    assert_eq!(pairs.iter().filter(|(k, _)| k == "ip").count(), 0, "{query}");
    assert_eq!(pairs.iter().filter(|(k, _)| k == "exp").count(), 1, "{query}");
}

#[test]
fn ipv6_binding_round_trips_percent_encoded() {
    let ip: IpAddr = "2001:db8::1".parse().unwrap();
    let k = keys("key-a");
    let query = k.sign("key-a", "/p", far_future(), Some(ip)).unwrap();
    assert!(query.contains("ip=2001%3Adb8%3A%3A1"), "{query}");
    let verifier = Verifier::signed_url(keys("key-a"));
    let peer: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
    assert_eq!(verify(&verifier, &format!("/p?{query}"), Some(peer)), AuthResult::Ok);
    let other: SocketAddr = "[2001:db8::2]:443".parse().unwrap();
    assert_eq!(verify(&verifier, &format!("/p?{query}"), Some(other)), AuthResult::Unauthorized);
}

#[test]
fn percent_encoded_kid_minted_elsewhere_verifies() {
    let k = keys("a-b");
    let query = k.sign("a-b", "/p", far_future(), None).unwrap();
    let hand_encoded = query.replace("kid=a-b", "kid=%61%2Db");
    assert_ne!(query, hand_encoded);
    let verifier = Verifier::signed_url(keys("a-b"));
    assert_eq!(verify(&verifier, &format!("/p?{hand_encoded}"), None), AuthResult::Ok);
}

#[test]
fn duplicate_kid_first_wins_and_bare_ip_key_is_rejected() {
    let k = keys("key-a");
    let query = k.sign("key-a", "/p", far_future(), None).unwrap();
    let verifier = Verifier::signed_url(keys("key-a"));
    assert_eq!(verify(&verifier, &format!("/p?{query}&kid=other"), None), AuthResult::Ok);
    // A bare `ip` key means "no value": it must not silently disable IP binding.
    assert_eq!(verify(&verifier, &format!("/p?{query}&ip"), None), AuthResult::Unauthorized);
}

/// Documented breaking difference: URLs minted by the old code with a literal
/// `+` in the kid are now read as a space and rejected.
#[test]
fn legacy_literal_plus_in_kid_is_no_longer_accepted() {
    let kid = "a+b";
    let query = keys(kid).sign(kid, "/p", far_future(), None).unwrap();
    let legacy = query.replace("kid=a%2Bb", "kid=a+b");
    let verifier = Verifier::signed_url(keys(kid));
    assert_eq!(verify(&verifier, &format!("/p?{query}"), None), AuthResult::Ok);
    assert_eq!(verify(&verifier, &format!("/p?{legacy}"), None), AuthResult::Unauthorized);
}
```

- [ ] **Step 2: Run on OLD code — FAIL**

```bash
cargo test -p broadcast-auth --all-features --locked --test signed_url_kid 2>&1 | grep -E 'test |test result|panicked|assert'
```

Expected FAIL: `kid_with_reserved_characters_round_trips` (`pairs.len()` is 5 or the verify is `Unauthorized`), `kid_cannot_inject_an_ip_binding_or_expiry` (an `ip` and a second `exp` appear), `ipv6_binding_round_trips_percent_encoded` (`ip=2001:db8::1` unencoded), `percent_encoded_kid_minted_elsewhere_verifies` (`Unauthorized`: no decoding), `duplicate_kid…` (`&ip` bare key → old ignores it ⇒ `Ok`, test expects `Unauthorized`), `legacy_literal_plus…` (the new-format `a%2Bb` query is `Unauthorized` on old code). Record in the report ("Revert-check evidence → Task 9, pre-fix run").

- [ ] **Step 3: Implement**

`Cargo.toml`: `form_urlencoded = "1"` (dependency-add procedure; it is already in the lock at 1.2.2). In `signed_url.rs`:
```rust
use std::borrow::Cow;

    pub fn sign(&self, kid: &str, path: &str, exp: u64, ip: Option<IpAddr>) -> Result<String> {
        let secret = self
            .secret_for(kid)
            .ok_or_else(|| Error::UnknownSignedUrlKeyId(kid.to_string()))?;
        let sig = URL_SAFE_NO_PAD.encode(hmac_sha256(secret, &canonical_string(path, exp, ip)));
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("exp", &exp.to_string());
        query.append_pair("kid", kid);
        query.append_pair("sig", &sig);
        if let Some(ip) = ip {
            query.append_pair("ip", &ip.to_string());
        }
        Ok(query.finish())
    }

/// The first value for `key` in an `application/x-www-form-urlencoded` query,
/// percent-decoded; a pair with no `=` has the value `""`.
fn query_get<'q>(query: &'q str, key: &str) -> Option<Cow<'q, str>> {
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}
```
In `verify`: `query_get(query, "exp").and_then(|s| s.parse::<u64>().ok())`, `.filter(|s| !s.is_empty())` for `kid`/`sig`, `keys.secret_for(&kid)`, `match query_get(query, "ip") { Some(raw) => raw.parse::<IpAddr>().map(Some).ok()?-style rejection … }` (a present-but-unparseable or empty `ip` returns `false`), `URL_SAFE_NO_PAD.decode(sig.as_bytes())`. Update the in-module test `query_get_skips_malformed_pairs_without_panicking` to the new contract: `query_get("a=1&garbage&b=2", "b").as_deref() == Some("2")`, `query_get("a=1&garbage&b=2", "garbage").as_deref() == Some("")`, never panics; note the changed assertion in the report. Update the module docs' wire-form block ("percent-encoded as `application/x-www-form-urlencoded`").

Update the IPv6 line of `tests/golden/signed_url_sign.txt` in this same commit: run the golden test, copy ONLY the changed `ip=2001%3Adb8%3A%3A1` line from the diff (the `key-a … 2001:db8::1` row; the sig is unchanged because the canonical string is unchanged). All other golden lines must stay byte-identical.

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo check -p multimux -p rtsp-runtime -p hls-runtime --all-features --locked 2>&1 | grep -E '^error|Finished'
```

Expected: all `ok` (`signed_url_kid` 6 passed, `golden_wire` with the one edited line, in-module signed_url tests); consumers `Finished`.

- [ ] **Step 5: Revert-check, then commit**

Revert `sign` to `format!("exp={exp}&kid={kid}&sig={sig}")` + `push_str(&format!("&ip={ip}"))` (keep the new `verify`), run `--test signed_url_kid`: `kid_with_reserved_characters_round_trips`, `kid_cannot_inject…` and `ipv6_binding…` FAIL; then revert only `verify`'s `query_get` to the `split('&')` version: `percent_encoded_kid_minted_elsewhere_verifies` FAILS. Record both, `git checkout -- broadcast-auth/src/signed_url.rs`, re-run PASS.

```bash
git add broadcast-auth/Cargo.toml Cargo.lock broadcast-auth/src/signed_url.rs broadcast-auth/tests/signed_url_kid.rs broadcast-auth/tests/golden/signed_url_sign.txt
git commit -m "fix(broadcast-auth)!: signed-URL query via form_urlencoded; kid and ip are percent-encoded (defect 6)"
```

---

### Task 10a: broadcast-auth — Digest `Authorization` fields via `http-auth`'s `ChallengeParser`

Skip this task and write escalation E0 if Task 1's step 2 failed on an RFC 7616 credential. Sites: `server.rs:659-686` (`strip_scheme` stays for now), `:673-690` (`split_digest_fields`, deleted), `:719` (`MAX_DIGEST_FIELDS`, kept), `:740-815` (`check_digest`, its field extraction at `:749-770`).

**Files:**
- Modify: `broadcast-auth/src/server.rs:673-690` (delete `split_digest_fields`), `:749-770` (use the new parser)
- Test: `broadcast-auth/tests/digest_params.rs` (new, public API only so the revert-check can restore `src/`; it uses the crate's own `md-5`/`hex` dependencies to hand-build headers the http-auth client cannot produce)

**Interfaces:** no public change. New private `fn parse_digest_fields(header: &str) -> Option<HashMap<String, String>>`: lower-cased parameter names → unescaped values; `None` for any syntax error, any scheme other than `Digest` (case-insensitive), more than one challenge/credentials list, more than `MAX_DIGEST_FIELDS` parameters, or a repeated parameter name.

- [ ] **Step 1: Failing tests**

```rust
//! Digest `Authorization` parsing contract (Review Focus #1).
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier, respond};
use md5::{Digest as _, Md5};

const REALM: &str = "cameras";

fn digest_verifier(user: &str, pw: &str) -> Verifier {
    Verifier::new(
        Credentials::Digest { username: user.into(), password: pw.into() },
        REALM,
    )
}

fn verify(v: &Verifier, header: &str, method: &str, uri: &str) -> AuthResult {
    let headers = [("authorization", header)];
    v.verify(&RequestContext::new(method, uri).with_headers(&headers))
}

/// A header the http-auth client produced for `user`/`pw` against `v`'s challenge.
fn answer(v: &Verifier, user: &str, pw: &str, method: &str, uri: &str) -> String {
    respond(&v.challenge(), &RequestContext::new(method, uri), Credentials::new(user, pw)).unwrap()
}

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

/// A hand-built, correctly-hashed `qop=auth`/MD5 header with `user` spelled
/// EXACTLY as given (the http-auth client refuses non-ASCII, so this is how a
/// browser's raw UTF-8 username is reproduced).
fn manual_header(v: &Verifier, user: &str, pw: &str, method: &str, uri: &str) -> String {
    let challenge = v.challenge();
    let start = challenge.find("nonce=\"").unwrap() + 7;
    let nonce = &challenge[start..start + challenge[start..].find('"').unwrap()];
    let (nc, cnonce) = ("00000001", "0a4f113b");
    let ha1 = md5_hex(&format!("{user}:{REALM}:{pw}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
    format!(
        "Digest username=\"{user}\", realm=\"{REALM}\", nonce=\"{nonce}\", uri=\"{uri}\", \
         algorithm=MD5, nc={nc}, cnonce=\"{cnonce}\", qop=auth, response=\"{response}\""
    )
}

#[test]
fn baseline_round_trip_still_verifies() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Ok);
}

/// http-auth escapes `"` and `\` as quoted-pairs; the old splitter had no
/// backslash handling and mis-read the value.
#[test]
fn username_with_quote_and_backslash_round_trips() {
    let user = r#"a"b\c"#;
    let v = digest_verifier(user, "pw");
    let h = answer(&v, user, "pw", "GET", "/x");
    assert!(h.contains(r#"username="a\"b\\c""#), "{h}");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Ok);
}

/// RFC 7235 §2.1: auth-param names are case-insensitive.
#[test]
fn parameter_names_are_case_insensitive() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let (scheme, rest) = h.split_once(' ').unwrap();
    let upper: Vec<String> = rest
        .split(", ")
        .map(|p| {
            let (k, val) = p.split_once('=').unwrap();
            format!("{}={val}", k.to_uppercase())
        })
        .collect();
    let header = format!("{scheme} {}", upper.join(", "));
    assert_eq!(verify(&v, &header, "GET", "/x"), AuthResult::Ok);
}

/// The old parser let the LAST duplicate win, so `username="evil", …,
/// username="admin"` was read as `admin` by the verifier while a proxy in
/// front may have logged `evil`. Reject ambiguity outright.
#[test]
fn a_repeated_parameter_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let smuggled = h.replacen("Digest ", "Digest username=\"evil\", ", 1);
    assert_eq!(verify(&v, &smuggled, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn a_second_challenge_after_the_credentials_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    assert_eq!(
        verify(&v, &format!("{h}, Basic realm=\"x\""), "GET", "/x"),
        AuthResult::Unauthorized
    );
}

/// DELIBERATE behaviour change (CHANGELOG, breaking): `http-auth`'s parser is
/// ASCII-only, so a raw UTF-8 `username` is a syntax error. RFC 7616 §3.4
/// wants `username*`/`userhash` for non-ASCII. See escalation E2.
#[test]
fn raw_non_ascii_username_is_rejected() {
    let v = digest_verifier("Jäs", "pw");
    let h = manual_header(&v, "Jäs", "pw", "GET", "/x");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn field_count_cap_still_applies() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let padding: String = (0..80).map(|i| format!(", x{i}=1")).collect();
    assert_eq!(verify(&v, &format!("{h}{padding}"), "GET", "/x"), AuthResult::Unauthorized);
}
```
(`hex` and `md-5` are normal dependencies of `broadcast-auth` — Task 8 added `hex`, `md-5` was already there — so an integration test may `use` them.)

- [ ] **Step 2: Run on OLD code**

```bash
cargo test -p broadcast-auth --all-features --locked --test digest_params 2>&1 | grep -E 'test |test result'
```

Expected: `baseline_round_trip…` and `field_count_cap…` PASS; FAIL: `username_with_quote_and_backslash_round_trips`, `parameter_names_are_case_insensitive`, `a_repeated_parameter_is_rejected` (old: last wins ⇒ `Ok`), `a_second_challenge_after_the_credentials_is_rejected` (old ignored the trailing junk ⇒ `Ok`), `raw_non_ascii_username_is_rejected` (old: `Ok`). Record in the report.

- [ ] **Step 3: Implement**

```rust
use http_auth::{ChallengeParser, ChallengeRef};

/// RFC 7235 `credentials` for the Digest scheme are an `auth-param` list — the
/// same grammar `http-auth` parses for challenges — so its parser reads them
/// (verified against the RFC 7616 §3.9.1 examples in
/// `tests/http_auth_rfc7616.rs`). Names are lower-cased (RFC 7235 §2.1),
/// quoted-pairs are unescaped, and anything ambiguous is refused: more than one
/// challenge, a scheme other than `Digest`, a repeated parameter, or more than
/// [`MAX_DIGEST_FIELDS`] parameters.
fn parse_digest_fields(header: &str) -> Option<HashMap<String, String>> {
    let mut parser = ChallengeParser::new(header);
    let challenge: ChallengeRef<'_> = parser.next()?.ok()?;
    if parser.next().is_some()
        || !challenge.scheme.eq_ignore_ascii_case("Digest")
        || challenge.params.len() > MAX_DIGEST_FIELDS
    {
        return None;
    }
    let mut fields = HashMap::with_capacity(challenge.params.len());
    for (name, value) in &challenge.params {
        if fields.insert(name.to_ascii_lowercase(), value.to_unescaped()).is_some() {
            return None;
        }
    }
    Some(fields)
}
```
In `check_digest` replace the `strip_scheme`/`split_digest_fields`/`HashMap` block (`:749-770`) by:
```rust
    let Some(fields) = parse_digest_fields(header) else {
        return DigestCheck::Reject;
    };
    let get = |k: &str| fields.get(k).map(String::as_str).unwrap_or_default();
```
Everything after (`get("username") != username …`) is unchanged. Delete `split_digest_fields` and update the `MAX_DIGEST_FIELDS` doc to say the cap is applied to the parsed parameter count. The doc comment of `check_digest` ("Rejects outright (without building the field map)…") is reworded: the parser still allocates one `Vec` entry per parameter, bounded by the header size the transport already caps.

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected: all `ok` — `digest_params` 7 passed, `http_auth_rfc7616`, every in-module Digest test (nc window, stale, wrong uri, qop/algorithm rejection).

- [ ] **Step 5: Revert-check, then commit**

Restore the old extraction block (from `git show HEAD~1:broadcast-auth/src/server.rs`, lines 749-770 and `split_digest_fields`), run `--test digest_params`: the five defect tests FAIL again; `git checkout -- broadcast-auth/src/server.rs`; PASS.

```bash
git add broadcast-auth/src/server.rs broadcast-auth/tests/digest_params.rs
git commit -m "refactor(broadcast-auth)!: parse Digest Authorization with http-auth ChallengeParser; reject duplicate params, honour quoted-pairs"
```

---

### Task 10b: broadcast-auth — digest-uri match via `url`

Site: `server.rs:841-851` `digest_uri_matches` (`split_once("://")` + `find('/')`). Review Focus #3.

**Files:**
- Modify: `broadcast-auth/Cargo.toml` (`url = "2"`), `broadcast-auth/src/server.rs:841-851`
- Test: `broadcast-auth/src/server.rs` `mod tests` (extend `digest_uri_matches_unit_cases` at `:1211`, add a sibling)

**Interfaces:** private `fn digest_uri_matches(client_uri: &str, request_uri: &str) -> bool`. Rule: identical strings match; otherwise `client_uri` must parse with `Url::parse`, already be in the parser's normalised spelling (`parsed.as_str() == client_uri` — so no dot-segments, upper-case host, default port or missing root `/`), and its path+query+fragment (`parsed[Position::BeforePath..]`) must equal `request_uri`.

- [ ] **Step 1: Tests**

```rust
    #[test]
    fn digest_uri_match_is_not_loosened_by_url_normalisation() {
        // The client hashed a spelling that is NOT the request-target, even
        // though the url crate would normalise it to one.
        assert!(!digest_uri_matches("http://h/a/../b", "/b"));
        assert!(!digest_uri_matches("http://h/%2e%2e/b", "/b"));
        assert!(!digest_uri_matches("http://h/./b", "/b"));
        assert!(!digest_uri_matches("HTTP://H/b", "/b"));
        assert!(!digest_uri_matches("http://h:80/b", "/b"));
        assert!(!digest_uri_matches("http://h", "/"));
        assert!(!digest_uri_matches("http://h/b#frag", "/b"));
        assert!(!digest_uri_matches("http://[::1/b", "/b"));
        // Normalised absolute-form still matches.
        assert!(digest_uri_matches("http://h/b?x=1&y=2", "/b?x=1&y=2"));
        assert!(digest_uri_matches("http://[2001:db8::1]:8080/b", "/b"));
        assert!(digest_uri_matches("rtsp://cam:554/live/ch1", "/live/ch1"));
        assert!(digest_uri_matches("rtsp://cam/live/ch1", "rtsp://cam/live/ch1"));
    }
```
The existing `digest_uri_matches_unit_cases` stays unchanged and must stay green.

- [ ] **Step 2: Run on OLD code**

```bash
cargo test -p broadcast-auth --all-features --locked --lib digest_uri_match 2>&1 | grep -E 'test |panicked|test result'
```

Expected: `digest_uri_match_is_not_loosened_by_url_normalisation` FAILS at `!digest_uri_matches("HTTP://H/b", "/b")` (old: scheme/host case ignored ⇒ true). Record. (The dot-segment asserts already pass on old code — they exist to catch a naive `Url` rewrite, see step 5.)

- [ ] **Step 3: Implement**

```rust
use url::{Position, Url};

fn digest_uri_matches(client_uri: &str, request_uri: &str) -> bool {
    if client_uri == request_uri {
        return true;
    }
    let Ok(parsed) = Url::parse(client_uri) else {
        return false;
    };
    parsed.as_str() == client_uri && &parsed[Position::BeforePath..] == request_uri
}
```
Rewrite the function's doc comment: keep the RFC 7230 §5.3 origin-form/absolute-form paragraph, replace the "first `/` after `://`" description with the rule above and the reason (normalisation must not widen the guard). Add the dependency (procedure; `url` 2.5.8 is already locked).

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

- [ ] **Step 5: Revert-check, then commit**

Temporarily change the last line to `&parsed[Position::BeforePath..] == request_uri` (drop the `as_str()` clause): `digest_uri_match_is_not_loosened_by_url_normalisation` must FAIL at `http://h/a/../b`. `git checkout -- broadcast-auth/src/server.rs`; PASS.

```bash
git add broadcast-auth/Cargo.toml Cargo.lock broadcast-auth/src/server.rs
git commit -m "refactor(broadcast-auth): digest-uri match via the url crate, normalised spelling only"
```

---

### Task 10c: broadcast-auth — Basic/Bearer via `headers::Authorization`, Bearer value hardening

Task 1 showed `ChallengeParser` cannot read `token68`, so Basic and Bearer use the spec's own fallback (`headers` typed `Authorization`). Sites: `server.rs:659-669` (`strip_scheme`, deleted), `:693-712` (`verify_basic`, `verify_bearer`), `authenticator.rs:62-66` (`format!("Bearer {token}")`). Real defect: a Bearer token containing CR/LF is emitted verbatim into the `Authorization` header (header injection, and RTSP is framed on raw CRLF).

**Files:**
- Modify: `broadcast-auth/Cargo.toml` (`headers = "0.4"`), `broadcast-auth/src/server.rs:659-712`, `broadcast-auth/src/authenticator.rs:62-66`, `broadcast-auth/src/error.rs` (new variant)
- Test: `broadcast-auth/tests/basic_bearer.rs` (new)

**Interfaces:**
- `Error::InvalidBearerToken` (`#[non_exhaustive]` enum, message `"bearer token is not a valid Authorization header value"`) returned by `Authenticator::authorization`/`respond` for a Bearer token carrying a byte that cannot appear in a header value.
- Verifier behaviour: scheme token matched case-insensitively and extra spaces tolerated (as before); Basic splits `user:password` at the FIRST `:` (RFC 7617 §2 forbids `:` in the user-id, so a configured username containing `:` can no longer match — CHANGELOG); a Bearer token is compared after trimming surrounding spaces; non-UTF-8 Basic payloads are `Unauthorized`.

- [ ] **Step 1: Failing + pin tests**

```rust
use broadcast_auth::{AuthResult, Authenticator, Credentials, Error, RequestContext, Verifier, respond};
use base64::Engine as _;

fn verify(v: &Verifier, header: &str) -> AuthResult {
    let headers = [("authorization", header)];
    v.verify(&RequestContext::new("GET", "/x").with_headers(&headers))
}

fn basic_verifier() -> Verifier {
    Verifier::new(
        Credentials::Basic { username: "admin".into(), password: "pa:ss".into() },
        "r",
    )
}

fn b64(s: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// Defect: header injection through the client's Bearer value.
#[test]
fn bearer_token_with_crlf_is_refused_by_the_client() {
    for bad in ["tok\r\nX-Evil: 1", "tok\nx", "tok\u{0}", "tök"] {
        let mut auth = Authenticator::from_challenge("", Credentials::bearer(bad)).unwrap();
        let err = auth.authorization(&RequestContext::new("GET", "/x")).unwrap_err();
        assert!(matches!(err, Error::InvalidBearerToken), "{bad:?}: {err:?}");
    }
    assert_eq!(
        respond("", &RequestContext::new("GET", "/x"), Credentials::bearer("abc.DEF-1_2~+/=")).unwrap(),
        "Bearer abc.DEF-1_2~+/="
    );
}

#[test]
fn basic_scheme_is_case_insensitive_and_password_may_contain_a_colon() {
    let v = basic_verifier();
    let payload = b64(b"admin:pa:ss");
    for scheme in ["Basic", "basic", "BASIC"] {
        assert_eq!(verify(&v, &format!("{scheme} {payload}")), AuthResult::Ok, "{scheme}");
    }
    assert_eq!(verify(&v, &format!("Basic   {payload}")), AuthResult::Ok);
    assert_eq!(verify(&v, &format!("Basic {}", b64(b"admin:wrong"))), AuthResult::Unauthorized);
    assert_eq!(verify(&v, &format!("Basic {}", b64(b"nobody:pa:ss"))), AuthResult::Unauthorized);
}

#[test]
fn basic_garbage_is_unauthorized_not_a_panic() {
    let v = basic_verifier();
    let non_utf8 = format!("Basic {}", b64(&[0xff, 0xfe, b':', 0x80]));
    for h in [
        "Basic", "Basic ", "Basic !!!", "Basic YWRtaW4=", // no colon
        non_utf8.as_str(),                                // not UTF-8
        "Bearer abc", "Digest x=1", "",
    ] {
        assert_eq!(verify(&v, h), AuthResult::Unauthorized, "{h:?}");
    }
}

#[test]
fn bearer_comparison_trims_and_is_exact() {
    let v = Verifier::new(Credentials::bearer("tok-123"), "r");
    assert_eq!(verify(&v, "Bearer tok-123"), AuthResult::Ok);
    assert_eq!(verify(&v, "bearer   tok-123"), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer tok-123 "), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer tok-1234"), AuthResult::Unauthorized);
    assert_eq!(verify(&v, "Bearer tok-12"), AuthResult::Unauthorized);
    assert_eq!(verify(&v, "Bearer"), AuthResult::Unauthorized);
}
```
(`base64` is a normal dependency of `broadcast-auth` (`Cargo.toml`), so the integration test can use it.)

- [ ] **Step 2: Run on OLD code**

```bash
cargo test -p broadcast-auth --all-features --locked --test basic_bearer 2>&1 | grep -E 'test |panicked|test result|error'
```

Expected: compile error `no variant … InvalidBearerToken` counts as the FAIL for the first test; to see the behavioural failure first, temporarily comment out the `matches!` arm and run: the old code returns `Ok("Bearer tok\r\nX-Evil: 1")` ⇒ `unwrap_err` panics. Record that. The other three tests PASS on old code (pins).

- [ ] **Step 3: Implement**

`error.rs`: add
```rust
    /// A Bearer token cannot be carried in an `Authorization` header value —
    /// it contains a byte outside visible ASCII (control character, CR/LF, or
    /// non-ASCII) — so no header was produced rather than a corrupt or
    /// injectable one.
    #[error("bearer token is not a valid Authorization header value")]
    InvalidBearerToken,
```
`authenticator.rs`:
```rust
use headers::{Authorization, Header};

            SchemeState::Bearer => {
                let Credentials::Bearer { token } = &self.credentials else {
                    unreachable!("SchemeState::Bearer only paired with Credentials::Bearer")
                };
                let typed = Authorization::bearer(token).map_err(|_| Error::InvalidBearerToken)?;
                let mut values: Vec<headers::HeaderValue> = Vec::with_capacity(1);
                typed.encode(&mut values);
                values
                    .first()
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
                    .ok_or(Error::InvalidBearerToken)
            }
```
(use `headers::HeaderValue`, not `http`; `headers::Header::encode` is `fn encode<E: Extend<HeaderValue>>(&self, values: &mut E)`, `Authorization::<Bearer>::bearer` returns `Result<Self, InvalidBearerToken>` — `headers-0.4.2/src/common/authorization.rs`.)

`server.rs`:
```rust
use headers::authorization::{Basic, Bearer, Credentials as HeaderCredentials};
use headers::{Authorization, Header, HeaderValue};

fn decode<C: HeaderCredentials>(header: &str) -> Option<Authorization<C>> {
    let value = HeaderValue::from_str(header).ok()?;
    Authorization::<C>::decode(&mut std::iter::once(&value)).ok()
}

/// RFC 7617 §2: decode the base64 payload and compare, in constant time.
fn verify_basic(header: &str, username: &str, password: &str) -> bool {
    let Some(auth) = decode::<Basic>(header) else {
        return false;
    };
    // Both compared unconditionally: no short-circuit on the user-id.
    let user_ok = constant_time_eq(auth.username().as_bytes(), username.as_bytes());
    let pass_ok = constant_time_eq(auth.password().as_bytes(), password.as_bytes());
    user_ok & pass_ok
}

/// RFC 6750 §2.1: compare the bearer token, in constant time.
fn verify_bearer(header: &str, token: &str) -> bool {
    let Some(auth) = decode::<Bearer>(header) else {
        return false;
    };
    constant_time_eq(auth.token().trim_end().as_bytes(), token.as_bytes())
}
```
Delete `strip_scheme` (and its RFC 7235 §2.1 comment — `headers` matches the scheme case-insensitively, `authorization.rs` `eq_ignore_ascii_case`). Follow the dependency-add procedure for `headers` (expect new lock packages: `headers`, `headers-core`, `httpdate`, `mime`, plus whatever `bytes`/`http` versions are already present).

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo check -p multimux -p rtsp-runtime -p hls-runtime --all-features --locked 2>&1 | grep -E '^error|Finished'
```

Expected: all `ok` (including the pre-existing `wrong_scheme_header_is_unauthorized`, `bearer_*`, scheme-case tests in `server.rs`); consumers `Finished` (an exhaustive `match` on `broadcast_auth::Error` in a consumer would show up here — `Error` is `#[non_exhaustive]`, so none is expected).

- [ ] **Step 5: Revert-check, then commit**

Replace the Bearer arm of `authorization` by the old `Ok(format!("Bearer {token}"))` (keep the new variant defined): `bearer_token_with_crlf_is_refused_by_the_client` FAILS (`unwrap_err` on `Ok`). `git checkout -- broadcast-auth/src/authenticator.rs`; PASS.

```bash
git add broadcast-auth/Cargo.toml Cargo.lock broadcast-auth/src/server.rs broadcast-auth/src/authenticator.rs broadcast-auth/src/error.rs broadcast-auth/tests/basic_bearer.rs
git commit -m "fix(broadcast-auth)!: Basic/Bearer through headers::Authorization; refuse Bearer tokens that cannot be a header value"
```

---

### Task 10d: broadcast-auth — `WWW-Authenticate` rendering: quoted-string escaping and CR/LF safety (escalation E1)

Task 1 established that no crate in the tree can render a challenge (`http-auth::ChallengeRef`/`ParamValue` have `Debug` only). The renderer therefore stays a formatter (documented exception E1), but its one real defect is fixed: `render_challenge` (`server.rs:550-565`) interpolates `realm` unescaped, so a realm containing `"`, `\`, CR or LF produces a malformed header or a response-splitting vector.

**Files:**
- Modify: `broadcast-auth/src/server.rs:550-565`
- Test: `broadcast-auth/tests/challenge_render.rs` (new)

**Interfaces:** `Verifier::challenge()`/`challenge_for()` still return `String`. The realm is rendered as an RFC 7230 §3.2.6 quoted-string: `"` and `\` become quoted-pairs, ASCII control characters are dropped. For an ordinary realm the output is byte-identical (the Task 2 golden `challenges.txt` must not change).

- [ ] **Step 1: Failing tests (the parser from Task 1 is the oracle)**

```rust
use broadcast_auth::{Credentials, Verifier};
use http_auth::ChallengeParser;

fn parse_one(header: &str) -> http_auth::ChallengeRef<'_> {
    let mut all = ChallengeParser::new(header)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("{header:?} is not a well-formed challenge: {e}"));
    assert_eq!(all.len(), 1, "{header:?}");
    all.remove(0)
}

fn realm_of(c: &http_auth::ChallengeRef<'_>) -> String {
    c.params.iter().find(|(k, _)| *k == "realm").unwrap().1.to_unescaped()
}

fn verifiers(realm: &str) -> [(&'static str, Verifier); 2] {
    [
        ("Basic", Verifier::new(Credentials::Basic { username: "u".into(), password: "p".into() }, realm)),
        ("Digest", Verifier::new(Credentials::Digest { username: "u".into(), password: "p".into() }, realm)),
    ]
}

#[test]
fn crlf_and_quote_in_the_realm_cannot_split_or_break_the_header() {
    for (scheme, v) in verifiers("x\"\r\nSet-Cookie: a=b") {
        let header = v.challenge();
        assert!(!header.contains('\r') && !header.contains('\n'), "{header:?}");
        let c = parse_one(&header);
        assert_eq!(c.scheme, scheme);
        assert_eq!(realm_of(&c), "x\"Set-Cookie: a=b");
    }
}

#[test]
fn backslash_comma_and_space_round_trip() {
    for (_, v) in verifiers(r#"Region, East \ "Beta""#) {
        assert_eq!(realm_of(&parse_one(&v.challenge())), r#"Region, East \ "Beta""#);
    }
}

#[test]
fn an_ordinary_realm_is_rendered_exactly_as_before() {
    let [(_, basic), (_, digest)] = verifiers("cameras");
    assert_eq!(basic.challenge(), "Basic realm=\"cameras\"");
    assert!(digest.challenge().starts_with("Digest realm=\"cameras\", nonce=\""));
}
```

- [ ] **Step 2: Run on OLD code — FAIL**

```bash
cargo test -p broadcast-auth --all-features --locked --test challenge_render 2>&1 | grep -E 'test |panicked|test result'
```

Expected: `crlf_and_quote…` and `backslash_comma…` FAIL (the old header contains `\r` / is not a well-formed challenge); `an_ordinary_realm…` PASSES. Record.

- [ ] **Step 3: Implement**

```rust
/// RFC 7230 §3.2.6 `quoted-string`: `"` and `\` become quoted-pairs; ASCII
/// control characters (CR, LF, NUL, …) cannot appear in a header value and are
/// dropped rather than letting a realm split the response.
fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars().filter(|c| !c.is_ascii_control()) {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}
```
and in `render_challenge`: `VerifierScheme::Basic { realm, .. } => format!("Basic realm={}", quoted(realm))`, and the Digest arm `format!("Digest realm={}, nonce=\"{nonce}\", qop=\"auth\", algorithm=MD5{stale}", quoted(realm))`. Add a doc line citing E1: "Rendering stays here because no crate in the workspace can produce a `WWW-Authenticate` value (`http_auth::ChallengeRef` implements only `Debug`); `tests/challenge_render.rs` parses every rendering back with `http-auth`'s `ChallengeParser`."

- [ ] **Step 4: Run — PASS, golden unchanged**

```bash
cargo test -p broadcast-auth --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo check -p multimux -p rtsp-runtime -p hls-runtime --all-features --locked 2>&1 | grep -E '^error|Finished'
```

Expected: all `ok`, including `golden_wire::challenges_match_golden` with NO golden edit.

- [ ] **Step 5: Revert-check, then commit**

Replace `quoted(realm)` by `format!("\"{realm}\"")` in the Basic arm: `crlf_and_quote…` FAILS; `git checkout -- broadcast-auth/src/server.rs`; PASS.

```bash
git add broadcast-auth/src/server.rs broadcast-auth/tests/challenge_render.rs
git commit -m "fix(broadcast-auth): escape the WWW-Authenticate realm as a quoted-string, drop control characters"
```

---

### Task 11: timed-metadata — `jiff` for RFC 3339 formatting (kills a `civil_from_days` copy, fixes range/overflow)

Site: `anchor.rs:1-56` (`format_rfc3339_ms`, `civil_from_days`, `TimeAnchor::{media_to_epoch_ms, rfc3339}`); caller `convert/daterange.rs:30-33`. Defects found: `format_rfc3339_ms(i64::MAX)` prints a 12-digit year; `media_to_epoch_ms` casts `u64 → i64` (wraps above `i64::MAX`, so the delta has the wrong sign) and adds without overflow protection (panic in debug builds).

Not breaking: `format_rfc3339_ms`/`TimeAnchor::rfc3339` keep their `-> String` signature (a `Result` would be a major-class change for every consumer — compliance-probe, hls-runtime, media-plane, caption-convert, multimux, media-doctor all depend on timed-metadata 0.5). They CLAMP to the representable range; new fallible twins report the error; the converter uses the fallible forms.

**Files:**
- Modify: `timed-metadata/Cargo.toml` (`jiff = { version = "0.2", default-features = false, features = ["alloc"] }`; add `"jiff/std"` to the `std` feature list)
- Modify: `timed-metadata/src/anchor.rs` (whole file), `timed-metadata/src/error.rs` (new variant), `timed-metadata/src/convert/daterange.rs:30-33`
- Test: `timed-metadata/tests/rfc3339_range.rs` (new, public API), existing unit tests in `anchor.rs:58-77` stay

**Interfaces:**
- `pub fn format_rfc3339_ms(epoch_ms: i64) -> String` — `YYYY-MM-DDTHH:MM:SS.sssZ` (always three fractional digits); input outside jiff's `Timestamp` range (years -9999..=9999) is clamped to the nearest representable instant.
- `pub fn try_format_rfc3339_ms(epoch_ms: i64) -> Result<String, Error>` — `Err(Error::TimestampOutOfRange(epoch_ms))` outside the range.
- `TimeAnchor::media_to_epoch_ms(&self, t: MediaTime) -> i64` — signature unchanged; computed in `i128`, saturating to `i64`.
- `TimeAnchor::rfc3339(&self, t) -> String` unchanged (clamps); new `TimeAnchor::try_rfc3339(&self, t) -> Result<String, Error>`.
- `Error::TimestampOutOfRange(i64)` — new variant on the `#[non_exhaustive]` enum; `scte35_to_daterange` returns it instead of writing a garbage `START-DATE`.

- [ ] **Step 1: Failing tests**

```rust
//! RFC 3339 range/overflow behaviour of the anchor (SP5, jiff).
use std::fs;

use broadcast_common::traits::Parse;
use scte35_splice::SpliceInfoSection;
use timed_metadata::anchor::{format_rfc3339_ms, try_format_rfc3339_ms};
use timed_metadata::convert::scte35_to_daterange;
use timed_metadata::daterange::DateRange;
use timed_metadata::event::{MediaTime, TimedEvent};
use timed_metadata::{Error, TimeAnchor};

/// `jiff::Timestamp::MIN` (-9999-01-01T00:00:00Z) in epoch milliseconds.
const MIN_MS: i64 = -377_705_116_800_000;
/// `jiff::Timestamp::MAX` truncated to milliseconds (9999-12-31T23:59:59.999Z).
const MAX_MS: i64 = 253_402_300_799_999;

#[test]
fn the_fallible_form_reports_out_of_range_epochs() {
    for ms in [i64::MAX, i64::MIN, MAX_MS + 1, MIN_MS - 1] {
        assert!(matches!(try_format_rfc3339_ms(ms), Err(Error::TimestampOutOfRange(v)) if v == ms), "{ms}");
    }
    assert_eq!(try_format_rfc3339_ms(MAX_MS).unwrap(), "9999-12-31T23:59:59.999Z");
    assert!(try_format_rfc3339_ms(MIN_MS).is_ok());
}

#[test]
fn the_infallible_form_clamps_instead_of_printing_a_garbage_year() {
    assert_eq!(format_rfc3339_ms(i64::MAX), "9999-12-31T23:59:59.999Z");
    assert_eq!(format_rfc3339_ms(i64::MIN), try_format_rfc3339_ms(MIN_MS).unwrap());
}

#[test]
fn media_time_arithmetic_saturates_instead_of_wrapping_or_panicking() {
    let hi = TimeAnchor { pts_90k: 0, utc_epoch_ms: i64::MAX };
    assert_eq!(hi.media_to_epoch_ms(MediaTime(u64::MAX)), i64::MAX);
    let lo = TimeAnchor { pts_90k: u64::MAX, utc_epoch_ms: i64::MIN };
    assert_eq!(lo.media_to_epoch_ms(MediaTime(0)), i64::MIN);
    // A PTS above i64::MAX must still count as LATER than one below it.
    let a = TimeAnchor { pts_90k: 0, utc_epoch_ms: 0 };
    assert!(a.media_to_epoch_ms(MediaTime(1 << 63)) > a.media_to_epoch_ms(MediaTime(1)));
}

#[test]
fn the_converter_errors_instead_of_emitting_an_unrepresentable_start_date() {
    let line = fs::read_to_string(format!(
        "{}/../fixtures/timed-metadata/daterange_2002.txt",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture");
    let dr = DateRange::parse_tag_line(line.trim()).unwrap();
    let raw = dr.scte35.unwrap().raw;
    let section = SpliceInfoSection::parse(&raw).unwrap();
    let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
    let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: i64::MAX };
    assert!(matches!(scte35_to_daterange(&ev, &anchor), Err(Error::TimestampOutOfRange(_))));
}

#[test]
fn well_known_instants_format_as_before() {
    assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(format_rfc3339_ms(-1), "1969-12-31T23:59:59.999Z");
    assert_eq!(format_rfc3339_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
    assert_eq!(format_rfc3339_ms(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
    // RFC 3339 §5.8 example "1985-04-12T23:20:50.52Z".
    assert_eq!(format_rfc3339_ms(482_196_050_520), "1985-04-12T23:20:50.520Z");
}
```
If `MIN_MS` is not `Timestamp::MIN.as_millisecond()` (the executor prints `jiff::Timestamp::MIN.as_millisecond()` once to confirm), correct the constant — it is a test input, not a behaviour.

- [ ] **Step 2: Run on OLD code — FAIL**

```bash
cargo test -p timed-metadata --all-features --locked --test rfc3339_range 2>&1 | grep -E 'error|test |panicked|test result'
```

Expected: compile error (`try_format_rfc3339_ms`, `Error::TimestampOutOfRange` do not exist) — the FAIL for this task. After stubbing those two items to `unimplemented!()` the behavioural failures are: the clamp test (`292277026596-12-04T…`), the saturation test (`attempt to add with overflow` panic in debug), the converter test.

- [ ] **Step 3: Implement**

`error.rs`:
```rust
    /// A wall-clock instant is outside the range RFC 3339 formatting supports
    /// (years -9999..=9999).
    #[error("epoch milliseconds {0} are outside the representable RFC 3339 range (years -9999..=9999)")]
    TimestampOutOfRange(i64),
```
`anchor.rs`:
```rust
//! Media-time ↔ wall-clock mapping for conversions that cross into UTC.
use crate::error::{Error, Result};
use crate::event::MediaTime;
use alloc::string::String;
use jiff::Timestamp;
use jiff::fmt::temporal::DateTimePrinter;

/// RFC 3339 with exactly three fractional digits (`…:SS.sssZ`).
const PRINTER: DateTimePrinter = DateTimePrinter::new().precision(Some(3));

/// Maps a known 90 kHz PTS to the UTC instant it represents (linear at 90 kHz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeAnchor {
    /// A reference PTS, in 90 kHz ticks.
    pub pts_90k: u64,
    /// The UTC time that `pts_90k` corresponds to, in milliseconds since the Unix epoch.
    pub utc_epoch_ms: i64,
}

impl TimeAnchor {
    /// Map a media instant to milliseconds since the Unix epoch, saturating at
    /// the `i64` limits.
    pub fn media_to_epoch_ms(&self, t: MediaTime) -> i64 {
        let delta_ticks = i128::from(t.0) - i128::from(self.pts_90k);
        let ms = i128::from(self.utc_epoch_ms) + delta_ticks * 1000 / i128::from(crate::PTS_HZ);
        ms.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// RFC 3339 UTC string (millisecond precision), clamped to the supported range.
    pub fn rfc3339(&self, t: MediaTime) -> String {
        format_rfc3339_ms(self.media_to_epoch_ms(t))
    }

    /// Like [`Self::rfc3339`], but an unrepresentable instant is an error.
    pub fn try_rfc3339(&self, t: MediaTime) -> Result<String> {
        try_format_rfc3339_ms(self.media_to_epoch_ms(t))
    }
}

/// Format milliseconds-since-epoch as `YYYY-MM-DDTHH:MM:SS.sssZ`; values outside
/// years -9999..=9999 are clamped (use [`try_format_rfc3339_ms`] to detect them).
pub fn format_rfc3339_ms(epoch_ms: i64) -> String {
    let clamped = epoch_ms.clamp(
        Timestamp::MIN.as_millisecond(),
        Timestamp::MAX.as_millisecond(),
    );
    let ts = Timestamp::from_millisecond(clamped).unwrap_or(Timestamp::UNIX_EPOCH);
    PRINTER.timestamp_to_string(&ts)
}

/// Fallible [`format_rfc3339_ms`].
pub fn try_format_rfc3339_ms(epoch_ms: i64) -> Result<String> {
    let ts = Timestamp::from_millisecond(epoch_ms).map_err(|_| Error::TimestampOutOfRange(epoch_ms))?;
    Ok(PRINTER.timestamp_to_string(&ts))
}
```
(`crate::PTS_HZ` is `pub const PTS_HZ: u64` at `lib.rs:43`; delete `civil_from_days` and the `format!`-based body.) `convert/daterange.rs:30-33`: `Some(t) => anchor.try_rfc3339(t)?`, `None => crate::anchor::try_format_rfc3339_ms(anchor.utc_epoch_ms)?`. The existing unit tests `epoch_zero_formats_unix_epoch` and `anchor_maps_media_to_wallclock` stay unchanged and must pass.

- [ ] **Step 4: Run — PASS, golden byte-identical, no_std build**

```bash
cargo test -p timed-metadata --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p timed-metadata --no-default-features --locked 2>&1 | tail -2
cargo check --locked --all-features -p compliance-probe -p hls-runtime -p media-plane -p caption-convert -p multimux -p media-doctor 2>&1 | grep -E '^error|Finished'
```

Expected: `golden_wire::rfc3339_formatting_matches_golden` passes with NO golden edit (all twelve values are inside the range; if jiff's `precision(Some(3))` prints `…:00Z` for a zero fraction the golden fails — then print through `DateTimePrinter::new().precision(Some(3))` via `print_timestamp` into a `String` is the same call; in that case record the jiff behaviour and format the fraction with `{:03}` of `ts.subsec_nanosecond() / 1_000_000` while keeping jiff for the calendar part). The consumer check compiles every dependent against the unchanged public API.

- [ ] **Step 5: Revert-check, then commit**

Replace the body of `format_rfc3339_ms` with a plain `try_format_rfc3339_ms(epoch_ms).unwrap_or_default()`: `the_infallible_form_clamps…` FAILS (empty string). Replace `media_to_epoch_ms` by the old `i64` arithmetic: `media_time_arithmetic_saturates…` FAILS/panics. `git checkout -- timed-metadata/src/anchor.rs`; PASS.

```bash
git add timed-metadata/Cargo.toml Cargo.lock timed-metadata/src/anchor.rs timed-metadata/src/error.rs timed-metadata/src/convert/daterange.rs timed-metadata/tests/rfc3339_range.rs
git commit -m "refactor(timed-metadata): jiff for RFC 3339 formatting; clamp/err on out-of-range instants, saturating anchor arithmetic"
```

---

### Task 12a: transmux — `jiff` for the CLI `availabilityStartTime`

Site: `cli.rs:70-105` (`availability_start_time_now`, `SECS_PER_DAY`, `civil_from_days` — a third copy of Hinnant's algorithm), call at `:1170`.

**Files:**
- Modify: `transmux/Cargo.toml` (`jiff = { version = "0.2", default-features = false, features = ["std"], optional = true }`; add `"dep:jiff"` to the `std` feature list)
- Modify: `transmux/src/cli.rs:70-105`, `:1170`
- Test: `transmux/src/cli.rs` (new `#[cfg(test)] mod availability_start_time_tests`)

**Interfaces:** private `fn availability_start_time(unix_secs: i64) -> String` (testable core: `YYYY-MM-DDTHH:MM:SSZ`, no fractional part) and `fn availability_start_time_now() -> String` (calls it with `jiff::Timestamp::now().as_second()`). `civil_from_days` and `SECS_PER_DAY` are deleted.

- [ ] **Step 1: Failing test**

```rust
#[cfg(test)]
mod availability_start_time_tests {
    use super::availability_start_time;

    #[test]
    fn formats_whole_seconds_utc_with_a_z_suffix() {
        assert_eq!(availability_start_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(availability_start_time(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(availability_start_time(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(availability_start_time(4_107_542_400), "2100-03-01T00:00:00Z");
        assert_eq!(availability_start_time(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn out_of_range_seconds_clamp_to_the_epoch_not_a_panic() {
        assert_eq!(availability_start_time(i64::MAX), "1970-01-01T00:00:00Z");
    }
}
```

- [ ] **Step 2: Run — FAIL (does not compile: `availability_start_time` missing)**

```bash
cargo test -p transmux --all-features --locked --lib availability_start_time_tests 2>&1 | grep -E '^error|test result'
```

- [ ] **Step 3: Implement**

```rust
fn availability_start_time(unix_secs: i64) -> String {
    let ts = jiff::Timestamp::from_second(unix_secs).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    // `Timestamp`'s Display is RFC 3339 UTC with a `Z` and, for a whole-second
    // instant, no fractional digits.
    format!("{ts}")
}

fn availability_start_time_now() -> String {
    availability_start_time(jiff::Timestamp::now().as_second())
}
```
(`cli.rs` is `std`-only, so `format!` is in scope.) Keep the existing doc comment about why "now at packaging time". Follow the dependency-add procedure.

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

- [ ] **Step 5: Commit**

```bash
git add transmux/Cargo.toml Cargo.lock transmux/src/cli.rs
git commit -m "refactor(transmux): jiff for the CLI availabilityStartTime"
```

---

### Task 12b: transmux — `xs:duration` parse via `jiff`

Site: `dash_parse.rs:785-870` (`parse_iso8601_duration`, `parse_fraction_nanos`, constants `SECONDS_PER_*`, `NANOSECOND_DIGITS`).

**Files:**
- Modify: `transmux/src/dash_parse.rs:785-870` (and delete the constants it alone used)
- Test: `transmux/tests/dash_duration.rs` (new, public API) + the existing unit tests at `dash_parse.rs:2218-2265` stay unchanged

**Interfaces:** `transmux::dash_parse::parse_iso8601_duration(&str) -> Result<core::time::Duration, DashParseError>` — unchanged signature and error (`DashParseError::InvalidDuration { value }`). Accepted: `PnDTnHnMnS` with a fractional seconds part up to nanoseconds (`jiff` `SpanParser`, ISO 8601 only, `days_are_24_hours`). Rejected as before: missing/lower-case `P`, bare `P`/`PT`, calendar units (years, months), negative, trailing garbage, friendly `1h 30m`. Documented widenings (CHANGELOG): `jiff` also accepts weeks (`P1W`) and a fractional part on the smallest unit of hours/minutes (`PT1.5H`); both are valid ISO 8601 durations.

- [ ] **Step 1: Characterisation tests (PASS on the old code, must still pass on jiff)**

```rust
use core::time::Duration;
use transmux::dash_parse::{DashParseError, parse_iso8601_duration};

fn ok(s: &str) -> Duration {
    parse_iso8601_duration(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
}

#[test]
fn xml_schema_duration_examples() {
    assert_eq!(ok("PT1H2M3.5S"), Duration::new(3723, 500_000_000));
    assert_eq!(ok("PT0S"), Duration::ZERO);
    assert_eq!(ok("PT2.0S"), Duration::from_secs(2));
    assert_eq!(ok("P1DT2H"), Duration::from_secs(93_600));
    assert_eq!(ok("P2D"), Duration::from_secs(172_800));
    assert_eq!(ok("PT36H"), Duration::from_secs(129_600));
    assert_eq!(ok("PT0.000000001S"), Duration::new(0, 1));
    assert_eq!(ok("  PT4S  "), Duration::from_secs(4));
    // XML Schema Part 2 §3.2.6's own examples (the ones without calendar units).
    assert_eq!(ok("P120D"), Duration::from_secs(10_368_000));
    assert_eq!(ok("PT1004199059S"), Duration::from_secs(1_004_199_059));
    assert_eq!(ok("PT130S"), Duration::from_secs(130));
    assert_eq!(ok("PT2M10S"), Duration::from_secs(130));
    assert_eq!(ok("P1DT2S"), Duration::from_secs(86_402));
}

#[test]
fn rejections_are_unchanged() {
    for bad in [
        "", "P", "PT", "T1S", "1H", "pt1s", "-PT1S", "+PT1S", "P1Y", "P1M", "P1Y2M", "P1Y1DT1S",
        "P1Y2M3DT10H30M", // XML Schema §3.2.6 example, calendar units
        "PT1S2", "PT1S garbage", "PTS", "PT-1S", "PT1,5S", "1h 30m", "PT99999999999999999999H",
    ] {
        assert!(
            matches!(parse_iso8601_duration(bad), Err(DashParseError::InvalidDuration { .. })),
            "{bad:?} must be an InvalidDuration"
        );
    }
}
```
(`DashParseError` and `parse_iso8601_duration` are `pub` in `dash_parse.rs`; confirm `DashParseError` is exported from the module path used above with `grep -n 'pub enum DashParseError' transmux/src/dash_parse.rs`.)

- [ ] **Step 2: Run on OLD code**

```bash
cargo test -p transmux --all-features --locked --test dash_duration 2>&1 | grep -E 'test |panicked|test result'
```

Expected: PASS (pins the old behaviour). If a listed rejection is NOT rejected by the old code (e.g. `PT1,5S`), remove that string and record it; do not edit the old code.

- [ ] **Step 3: Implement**

```rust
pub fn parse_iso8601_duration(s: &str) -> Result<Duration> {
    let trimmed = s.trim();
    let invalid = || DashParseError::InvalidDuration { value: trimmed.to_string() };
    // jiff's parser also takes a lower-case `p` and a leading sign; xs:duration
    // as MPDs use it is an upper-case `P…` with no sign.
    if !trimmed.starts_with('P') {
        return Err(invalid());
    }
    let span = jiff::fmt::temporal::SpanParser::new()
        .parse_span(trimmed)
        .map_err(|_| invalid())?;
    // Days (and weeks) count as 24 h; years and months are calendar units that
    // need a reference date, so `to_duration` refuses them — as before.
    let signed = span
        .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
        .map_err(|_| invalid())?;
    Duration::try_from(signed).map_err(|_| invalid())
}
```
Delete `parse_fraction_nanos` and `SECONDS_PER_DAY`/`SECONDS_PER_HOUR`/`SECONDS_PER_MINUTE`/`NANOSECOND_DIGITS` if nothing else uses them (the compiler reports). Keep the doc comment, adjusting the "Only a plain day count" sentence to the widening note above. `Duration::try_from(SignedDuration)` rejects negatives (`jiff` implements `TryFrom<SignedDuration> for core::time::Duration`, `signed_duration.rs:2574`).

- [ ] **Step 4: Run — PASS**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected: `dash_duration`, the in-module `iso8601_duration_*` tests (`dash_parse.rs:2218-2265`), `dash_parse` integration tests and the `parse-*.txt` golden Debug dumps all pass. If a listed rejection now parses (e.g. `PT1S2`), STOP: it is a jiff widening beyond the two documented — add a narrow pre-check and a test, do not loosen the list.

- [ ] **Step 5: Commit**

```bash
git add transmux/src/dash_parse.rs transmux/tests/dash_duration.rs
git commit -m "refactor(transmux): xs:duration parse via jiff SpanParser (calendar units, sign and lower-case still rejected)"
```

---

### Task 12c: transmux — `xs:duration` build via `jiff`, with the listed golden difference

Sites: `dash.rs:722` (`"PT2.0S"` literal), `:763` (`xs_duration_tenths(max_tenths)`), `:771` (`"PT0.0S"` literal), `:1119-1123` (`xs_duration_tenths`). `format!("PT{}.{}S", …)` always prints one decimal; jiff prints the shortest ISO 8601 form. This is the one intended output difference of Task 12.

**Files:**
- Modify: `transmux/src/dash.rs:722,763,771,1119-1123`
- Modify (golden, same commit): `transmux/tests/golden/dash-default.mpd`, `dash-dynamic.mpd`, `dash-protected.mpd`, `dash-timeline.mpd`, `dash-trick.mpd`, `lldash-plain.mpd`, `lldash-rate.mpd`, `lldash-timeline.mpd`, `lldash-utc.mpd`
- Test: `transmux/tests/golden.rs` (existing, byte-for-byte) + `transmux/src/dash.rs` unit test

**Interfaces:** private `fn xs_duration(d: core::time::Duration) -> String` (`jiff::fmt::temporal::SpanPrinter` over `jiff::SignedDuration`). Output differences, each to be listed in the CHANGELOG `### Changed` section with this example: `minBufferTime="PT2.0S"` → `"PT2S"`, `Period@start="PT0.0S"` → `"PT0S"`, `mediaPresentationDuration="PT3.0S"` → `"PT3S"`, and durations of a minute or more regroup (`"PT90.0S"` → `"PT1M30S"`). All are equal xs:durations.

- [ ] **Step 1: Failing test**

```rust
    #[test]
    fn xs_duration_is_the_shortest_iso_8601_form() {
        use core::time::Duration;
        assert_eq!(xs_duration(Duration::from_secs(2)), "PT2S");
        assert_eq!(xs_duration(Duration::ZERO), "PT0S");
        assert_eq!(xs_duration(Duration::from_millis(2500)), "PT2.5S");
        assert_eq!(xs_duration(Duration::from_secs(90)), "PT1M30S");
        assert_eq!(xs_duration(Duration::from_secs(3600)), "PT1H");
        // Round trip through the parser (Task 12b).
        for d in [Duration::from_millis(100), Duration::from_secs(7200), Duration::new(61, 500_000_000)] {
            assert_eq!(crate::dash_parse::parse_iso8601_duration(&xs_duration(d)).unwrap(), d);
        }
    }
```

- [ ] **Step 2: Run — FAIL (`xs_duration` missing)**, then **Step 3: Implement**

```rust
fn xs_duration(d: core::time::Duration) -> String {
    let signed = jiff::SignedDuration::try_from(d).unwrap_or(jiff::SignedDuration::MAX);
    jiff::fmt::temporal::SpanPrinter::new().duration_to_string(&signed)
}
```
Replace the call sites: `("minBufferTime", xs_duration(Duration::from_secs(2)))`, `xs_duration(Duration::from_millis(max_tenths.saturating_mul(100)))`, `("start", xs_duration(Duration::ZERO))`; delete `xs_duration_tenths` (its comment "integer-only for `no_std`" is moot: `dash` is `std`-only). `use core::time::Duration;` at the top of `dash.rs` if absent.

- [ ] **Step 4: See exactly which golden bytes move, then apply ONLY those**

```bash
cargo test -p transmux --all-features --locked --test golden 2>&1 | grep -E 'differs|test result'
```

Expected: `dash_mpd_variants_match_golden` (and the LL-DASH one) FAIL with a diff limited to `PT2.0S`/`PT0.0S`/`PT3.0S` attribute values. Apply:

```bash
cd transmux/tests/golden
sed -i '' -e 's/"PT2\.0S"/"PT2S"/g' -e 's/"PT0\.0S"/"PT0S"/g' -e 's/"PT3\.0S"/"PT3S"/g' dash-*.mpd lldash-*.mpd
git diff --stat . ; git diff . | grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' | sort | uniq -c
cd ../../..
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected: the diff shows ONLY `PT2.0S→PT2S`, `PT0.0S→PT0S`, `PT3.0S→PT3S` line pairs (the `parse-*.txt`/`smooth*` goldens are untouched — they print parse results of input fixtures, which this task does not change). All suites pass. If the printer yields anything other than those three strings in a golden (e.g. `PT3S` for a value that was `PT2.5S`), hand-edit exactly that value and add it to the CHANGELOG list — never `GOLDEN_BLESS`.

- [ ] **Step 5: Downstream check, then commit**

```bash
grep -rn 'PT[0-9]*\.[0-9]S' multimux/tests media-doctor/tests hls-runtime/tests 2>/dev/null | head
cargo check -p multimux -p media-doctor --all-features --locked 2>&1 | grep -E '^error|Finished'
git add transmux/src/dash.rs transmux/tests/golden/dash-default.mpd transmux/tests/golden/dash-dynamic.mpd transmux/tests/golden/dash-protected.mpd transmux/tests/golden/dash-timeline.mpd transmux/tests/golden/dash-trick.mpd transmux/tests/golden/lldash-plain.mpd transmux/tests/golden/lldash-rate.mpd transmux/tests/golden/lldash-timeline.mpd transmux/tests/golden/lldash-utc.mpd
git commit -m "refactor(transmux)!: xs:duration output via jiff (PT2.0S becomes PT2S); goldens updated, differences listed in CHANGELOG"
```

The first command should print nothing that asserts on TRANSMUX-generated durations (multimux's own `PT…S` sites are W2). Any hit that does, goes into the report for the orchestrator, not fixed here.

---

### Task 13: SP3 — delete `transmux/src/uri.rs`; BaseURL resolution via `url`

Site: `uri.rs` (421 lines: `UriReference`, `resolve`, `merge`, `remove_dot_segments`, the §5.4 tables at `:305-354`, `resolve_segment`, `first_forbidden_char`, `try_resolve*`), module decl `lib.rs:204`, re-exports `lib.rs:393-396`, callers `dash_parse.rs:996-1028` (`resolve_segment_url`, `try_resolve_segment_url`), tests `tests/uri.rs`, `tests/dash_parse.rs:599,764`. Review Focus #4.

**Files:**
- Create: `transmux/src/base_url.rs`, `transmux/tests/base_url.rs`
- Delete: `transmux/src/uri.rs`, `transmux/tests/uri.rs`
- Modify: `transmux/Cargo.toml` (`url = { version = "2", optional = true }`; `"dep:url"` into the `std` feature list), `transmux/src/lib.rs:204` (`pub mod uri;` → `#[cfg(feature = "std")] pub mod base_url;`), `:393-396` (re-exports), `transmux/src/dash_parse.rs:996-1028`, `transmux/tests/dash_parse.rs:599,764`

**Interfaces (new, `std` only; `url::Url` appears in the public API, so this is part of transmux's major-class bump):**
```rust
pub fn first_forbidden_char(s: &str) -> Option<char>;
pub fn resolve(base: Option<&Url>, reference: &str) -> Option<String>;
pub fn resolve_chain(base: Option<&Url>, chain: &[String], reference: &str) -> Option<String>;
// Mpd (dash_parse.rs): the fallible method replaces BOTH old ones
pub fn resolve_segment_url(&self, mpd_url: Option<&Url>, period: &Period,
    adaptation_set: &AdaptationSet, representation: &Representation, reference: &str) -> Option<String>;
// crate root: pub use base_url::{resolve as resolve_url_reference, resolve_chain as resolve_base_url_chain};
```
Removed: `transmux::uri` (all items), `resolve_uri_reference`, `resolve_uri_segment`, `try_resolve_uri_reference`, `Mpd::try_resolve_segment_url`, and the infallible `Mpd::resolve_segment_url`'s old 4-argument form.

Semantics:
- `base = Some(url)`: the MPD's own URL (an `http(s)` source URL, or `Url::from_file_path(path)` for a file). `base = None` (in-memory input): the fixed synthetic base `transmux-relative:///transmux-relative-root/`.
- Chain entries (trimmed, empty ones skipped) are joined in order, so an absolute entry resets the chain; the reference is joined last.
- The synthetic base is stripped in exactly ONE place, `render(&Url) -> String`: a result still under the synthetic root becomes the relative remainder; `///x` becomes `/x` (absolute path); `//host/x` stays `//host/x`; anything else is the URL's own text.
- `None` is returned for input containing a control character or whitespace (checked BEFORE joining: the WHATWG parser silently deletes tab/CR/LF, so `Url::join("a\r\nHost: x")` would otherwise yield a clean-looking `aHost:%20x`) and for any `Url::join` error.

- [ ] **Step 1: Write the failing tests**

First pull the §5.4 tables out of the file being deleted so they move verbatim:
```bash
mkdir -p target
git show HEAD:transmux/src/uri.rs | sed -n '/^pub const RFC3986_NORMAL_EXAMPLES:/,/^];/p'   > target/normal.rs
git show HEAD:transmux/src/uri.rs | sed -n '/^pub const RFC3986_ABNORMAL_EXAMPLES:/,/^];/p' > target/abnormal.rs
```
`transmux/tests/base_url.rs`:
```rust
//! RFC 3986 §5.4 reference-resolution vectors and BaseURL-chain behaviour for
//! `transmux::base_url` (SP3). The two tables moved here verbatim from the
//! deleted `src/uri.rs`; the `url` crate implements the WHATWG URL algorithm,
//! which differs from strict RFC 3986 in exactly the rows listed in
//! `WHATWG_DIFFERENCES`.
#![cfg(feature = "std")]

use transmux::base_url::{first_forbidden_char, resolve, resolve_chain};
use url::Url;

const BASE: &str = "http://a/b/c/d;p?q";

// paste the NORMAL table (23 rows) and the ABNORMAL table (18 rows) here, as
// `const RFC3986_NORMAL_EXAMPLES: &[(&str, &str)] = &[ … ];` / `RFC3986_ABNORMAL_EXAMPLES`

/// Rows where WHATWG (the `url` crate) legitimately differs from RFC 3986:
/// `//g` serialises with a root path; the same-scheme reference `http:g` is
/// treated as relative for special schemes (§5.4.2's own "strict parser" note).
const WHATWG_DIFFERENCES: [(&str, &str); 2] = [("//g", "http://g/"), ("http:g", "http://a/b/c/g")];

fn run(rows: &[(&str, &str)]) {
    let base = Url::parse(BASE).unwrap();
    for (reference, rfc) in rows {
        let expected = WHATWG_DIFFERENCES
            .iter()
            .find(|(r, _)| r == reference)
            .map_or(*rfc, |(_, w)| *w);
        assert_eq!(resolve(Some(&base), reference).as_deref(), Some(expected), "{reference:?}");
    }
}

#[test]
fn rfc3986_5_4_1_normal_examples() {
    assert_eq!(RFC3986_NORMAL_EXAMPLES.len(), 23);
    run(RFC3986_NORMAL_EXAMPLES);
}

#[test]
fn rfc3986_5_4_2_abnormal_examples() {
    assert_eq!(RFC3986_ABNORMAL_EXAMPLES.len(), 18);
    run(RFC3986_ABNORMAL_EXAMPLES);
    let base = Url::parse(BASE).unwrap();
    assert_eq!(resolve(Some(&base), "http:g").as_deref(), Some("http://a/b/c/g"));
}

#[test]
fn base_chain_each_level_resolves_against_the_one_before() {
    let chain = vec![
        "https://cdn.example.com/vod/".to_string(),
        "period-1/".to_string(),
        "video/".to_string(),
    ];
    assert_eq!(
        resolve_chain(None, &chain, "seg-1.m4s").as_deref(),
        Some("https://cdn.example.com/vod/period-1/video/seg-1.m4s")
    );
    assert_eq!(
        resolve_chain(None, &chain, "../audio/seg-1.m4s").as_deref(),
        Some("https://cdn.example.com/vod/period-1/audio/seg-1.m4s")
    );
    let reset = vec!["https://cdn.example.com/vod/".to_string(), "https://other.example.net/".to_string()];
    assert_eq!(resolve_chain(None, &reset, "seg.m4s").as_deref(), Some("https://other.example.net/seg.m4s"));
    assert_eq!(resolve_chain(None, &[], "https://x/y.m4s").as_deref(), Some("https://x/y.m4s"));
    // Empty / whitespace-only BaseURLs contribute nothing.
    let blanks = vec![String::new(), "  ".to_string(), "https://h/d/".to_string()];
    assert_eq!(resolve_chain(None, &blanks, "s.m4s").as_deref(), Some("https://h/d/s.m4s"));
}

#[test]
fn trailing_slash_is_significant() {
    let mpd = Url::parse("https://cdn.example.com/vod/index.mpd").unwrap();
    assert_eq!(resolve(Some(&mpd), "seg.m4s").as_deref(), Some("https://cdn.example.com/vod/seg.m4s"));
    let dir = Url::parse("https://cdn.example.com/vod/").unwrap();
    assert_eq!(resolve(Some(&dir), "seg.m4s").as_deref(), Some("https://cdn.example.com/vod/seg.m4s"));
    let file = Url::parse("https://cdn.example.com/vod").unwrap();
    assert_eq!(resolve(Some(&file), "seg.m4s").as_deref(), Some("https://cdn.example.com/seg.m4s"));
}

#[test]
fn the_mpd_url_or_a_file_url_is_the_base() {
    let mpd = Url::parse("https://cdn.example.com/vod/index.mpd").unwrap();
    let chain = vec!["period-1/".to_string()];
    assert_eq!(resolve_chain(Some(&mpd), &chain, "seg.m4s").as_deref(), Some("https://cdn.example.com/vod/period-1/seg.m4s"));
    let file = Url::from_file_path("/media/a/index.mpd").unwrap();
    assert_eq!(resolve(Some(&file), "seg.m4s").as_deref(), Some("file:///media/a/seg.m4s"));
}

// --- the synthetic base (option (a)) -----------------------------------------

#[test]
fn relative_results_stay_relative_when_there_is_no_base() {
    assert_eq!(resolve(None, "seg/1.m4s").as_deref(), Some("seg/1.m4s"));
    assert_eq!(resolve(None, "seg.m4s?x=1#f").as_deref(), Some("seg.m4s?x=1#f"));
    assert_eq!(resolve_chain(None, &["video/".to_string()], "seg-1.m4s").as_deref(), Some("video/seg-1.m4s"));
    assert_eq!(resolve_chain(None, &["a/".to_string(), "../b/".to_string()], "s.m4s").as_deref(), Some("b/s.m4s"));
}

#[test]
fn an_absolute_path_reference_is_not_mistaken_for_a_relative_one() {
    assert_eq!(resolve(None, "/abs/x.m4s").as_deref(), Some("/abs/x.m4s"));
    assert_eq!(resolve(None, "//cdn.example/x.m4s").as_deref(), Some("//cdn.example/x.m4s"));
    assert_eq!(resolve(None, "http://x/y").as_deref(), Some("http://x/y"));
}

/// RFC 3986 §5.2.4 drops `..` segments that climb above the root, so a
/// relative reference that escapes the (synthetic) base comes back as an
/// absolute path. The old resolver returned `x`. Documented difference.
#[test]
fn climbing_above_the_synthetic_root_yields_an_absolute_path() {
    assert_eq!(resolve(None, "../x").as_deref(), Some("/x"));
}

#[test]
fn non_ascii_references_are_percent_encoded_by_the_url_crate() {
    assert_eq!(resolve(None, "é.m4s").as_deref(), Some("%C3%A9.m4s"));
    let base = Url::parse("http://a/b/").unwrap();
    assert_eq!(resolve(Some(&base), "é.m4s").as_deref(), Some("http://a/b/%C3%A9.m4s"));
}

// --- hostile input -------------------------------------------------------------

/// WHY the guard must run before joining: the WHATWG parser deletes tab, CR and
/// LF instead of rejecting them.
#[test]
fn the_url_crate_silently_strips_crlf_so_the_guard_cannot_be_delegated() {
    assert_eq!(Url::parse("http://a/b\r\nc").unwrap().as_str(), "http://a/bc");
}

#[test]
fn control_characters_and_whitespace_are_rejected_before_parsing() {
    let base = Url::parse("http://a/b/").unwrap();
    for evil in [
        "seg\r\nHost: evil.example", "seg\nX: y", "seg\rX: y", "seg\tX: y", "seg X: y",
        "\u{0}", "seg\u{7f}", "seg\u{1b}[31m",
    ] {
        assert!(first_forbidden_char(evil).is_some(), "{evil:?}");
        assert_eq!(resolve(Some(&base), evil), None, "{evil:?}");
        assert_eq!(resolve(None, evil), None, "{evil:?}");
    }
    assert_eq!(resolve_chain(None, &["http://a/\nb/".to_string()], "s"), None);
    assert_eq!(resolve_chain(None, &["x/".to_string()], "s\r\nHost: e"), None);
    assert_eq!(resolve(Some(&base), "seg.m4s").as_deref(), Some("http://a/b/seg.m4s"));
    // A percent-encoded control character is not raw text and passes through.
    assert_eq!(resolve(Some(&base), "seg%0D%0AHost:evil").as_deref(), Some("http://a/b/seg%0D%0AHost:evil"));
}

#[test]
fn unparseable_references_are_none_not_a_panic() {
    let base = Url::parse("http://a/b/").unwrap();
    for bad in ["http://[::1", "http://", "http://a:99999999/", "http://exa mple/"] {
        assert_eq!(resolve(Some(&base), bad), None, "{bad:?}");
    }
    // Backslash is a path separator for special schemes (WHATWG): pinned.
    assert_eq!(resolve(Some(&base), "x\\y.m4s").as_deref(), Some("http://a/b/x/y.m4s"));
}

#[test]
fn crate_root_re_exports_exist() {
    assert_eq!(transmux::resolve_url_reference(Some(&Url::parse("http://a/b/c/d;p?q").unwrap()), "g").as_deref(), Some("http://a/b/c/g"));
    assert_eq!(transmux::resolve_base_url_chain(None, &["http://a/".to_string()], "s.m4s").as_deref(), Some("http://a/s.m4s"));
}
```
(Fill the two table consts from the extracted files; that is a copy, not new content. `"http://exa mple/"` contains a space and is caught by the guard before the parser.)

- [ ] **Step 2: Run — FAIL (module does not exist)**

```bash
cargo test -p transmux --all-features --locked --test base_url 2>&1 | grep -E '^error|test result' | head -3
```

- [ ] **Step 3: Implement `base_url.rs`**

```rust
//! BaseURL and URL-reference resolution (ISO/IEC 23009-1 §5.6.5 over RFC 3986
//! §5, as implemented by the `url` crate — SP3). `std` only.
//!
//! A DASH client turns a relative `SegmentTemplate`/`BaseURL` reference into the
//! URL it fetches by joining a `BaseURL` chain onto the MPD's own URL. With no
//! location (an in-memory MPD) the join happens against a fixed synthetic base,
//! [`SYNTHETIC_BASE`], and [`render`] — the ONE place that knows about it —
//! strips it again from results that stayed relative.
//!
//! The `url` crate implements the WHATWG URL algorithm; it differs from strict
//! RFC 3986 in two §5.4 rows (see `tests/base_url.rs`).

use url::Url;

/// Scheme of the synthetic base (never leaves this module's results).
const SYNTHETIC_SCHEME: &str = "transmux-relative";
/// The synthetic base: an unlikely directory so that an absolute-path
/// reference (`/x`) cannot be confused with a relative one that stayed under
/// it. A reference that literally starts with this directory name collides;
/// no real MPD does.
pub const SYNTHETIC_BASE: &str = "transmux-relative:///transmux-relative-root/";

pub fn first_forbidden_char(s: &str) -> Option<char> {
    s.chars().find(|c| c.is_control() || c.is_whitespace())
}

pub fn resolve(base: Option<&Url>, reference: &str) -> Option<String> {
    resolve_chain(base, &[], reference)
}

pub fn resolve_chain(base: Option<&Url>, chain: &[String], reference: &str) -> Option<String> {
    if first_forbidden_char(reference).is_some()
        || chain.iter().any(|b| first_forbidden_char(b).is_some())
    {
        return None;
    }
    let mut current = match base {
        Some(b) => b.clone(),
        None => Url::parse(SYNTHETIC_BASE).ok()?,
    };
    for entry in chain.iter().map(|b| b.trim()).filter(|b| !b.is_empty()) {
        current = current.join(entry).ok()?;
    }
    Some(render(&current.join(reference).ok()?))
}

/// The single place the synthetic base is stripped.
pub fn render(url: &Url) -> String {
    let text = url.as_str();
    let Some(rest) = text.strip_prefix(SYNTHETIC_SCHEME).and_then(|r| r.strip_prefix(':')) else {
        return text.to_owned();
    };
    if let Some(relative) = rest.strip_prefix("///transmux-relative-root/") {
        return relative.to_owned();
    }
    match rest.strip_prefix("//") {
        Some(path) if path.starts_with('/') => path.to_owned(),
        _ => rest.to_owned(),
    }
}
```
with doc comments on every `pub fn` (`first_forbidden_char`: the CR/LF guard that replaces `uri.rs`'s; `resolve`/`resolve_chain`: the rules above; `render`: the strip rules) and unit tests in the module: `synthetic_base_parses` (`Url::parse(SYNTHETIC_BASE).is_ok()` — the `.ok()?` in `resolve_chain` can then never fire) and `render_cases` (the four stripping cases through `Url::parse` inputs: under-root → relative, `///x` → `/x`, `//h/x` → `//h/x`, a non-synthetic URL → unchanged).

`dash_parse.rs:996-1028` becomes:
```rust
    pub fn resolve_segment_url(
        &self,
        mpd_url: Option<&url::Url>,
        period: &Period,
        adaptation_set: &AdaptationSet,
        representation: &Representation,
        reference: &str,
    ) -> Option<String> {
        crate::base_url::resolve_chain(
            mpd_url,
            &self.base_url_chain(period, adaptation_set, representation),
            reference,
        )
    }
```
(delete `try_resolve_segment_url` and rewrite the doc: `mpd_url` is the MPD's own URL — source URL or `Url::from_file_path` — `None` for in-memory input; `None` result means the reference or a BaseURL carries a control character/whitespace or does not parse.) `lib.rs`: replace `pub mod uri;` by `#[cfg(feature = "std")] pub mod base_url;` and the `pub use uri::{…}` block by `#[cfg(feature = "std")] pub use base_url::{resolve as resolve_url_reference, resolve_chain as resolve_base_url_chain};`. Update `tests/dash_parse.rs:599,764` to `mpd.resolve_segment_url(None, period, set, repr, reference).expect("resolves")`. `git rm transmux/src/uri.rs transmux/tests/uri.rs`. Port, do not drop, the behaviour of every deleted test that is not about the removed APIs: the chain/trailing-slash/CR-LF/re-export tests are in the new file above; the deleted ones tested `UriReference`, `merge`, `remove_dot_segments` (now `url`'s job) and "never panics" over `resolve` — covered by `unparseable_references_are_none_not_a_panic`. List the deleted test names in the report.

- [ ] **Step 4: Run — PASS everywhere**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p transmux --no-default-features --locked 2>&1 | tail -2
cargo check -p multimux -p hls-runtime -p media-doctor --all-features --locked 2>&1 | grep -E '^error|Finished'
grep -rn 'uri::\|UriReference\|resolve_uri_' --include=*.rs --include=*.md transmux multimux hls-runtime media-doctor docs 2>/dev/null | grep -v CHANGELOG | head
```

Expected: all `ok`; both builds `Finished`; the grep is empty apart from CHANGELOG history (fix any README/doc mention).

- [ ] **Step 5: Revert-check the guard, then commit**

Move the `first_forbidden_char` check in `resolve_chain` AFTER the joins (so `Url` sees the raw text): `control_characters_and_whitespace_are_rejected_before_parsing` FAILS (`Some("http://a/b/segHost:evil.example")`-style cleaned output instead of `None`). Restore with `git checkout -- transmux/src/base_url.rs` if committed first; PASS.

```bash
git add transmux/Cargo.toml Cargo.lock transmux/src/base_url.rs transmux/src/lib.rs transmux/src/dash_parse.rs transmux/tests/base_url.rs transmux/tests/dash_parse.rs
git rm -q transmux/src/uri.rs transmux/tests/uri.rs
git commit -m "refactor(transmux)!: delete uri.rs; BaseURL resolution via url::Url::join with a stripped synthetic base"
```

---

### Task 14: SP4 — transmux RTP SDP built through `sdp-types` 0.2 (`std`-only)

Sites: `rtp.rs:231-237` (`RtpOutput.sdp`), `:379-467` (`package`), `:810-912` (`build_sdp`, `build_sdp_with_connection`, `sdp_video`, `sdp_audio`), `lib.rs:337` (re-export of `build_sdp_with_connection`). fmtp stays codec logic (owner decision): the `fmtp` attribute VALUE is built as a string exactly as today, because `sdp_types::Fmtp`'s `Display` joins parameters with `;` and no space (`attributes.rs:380-398`), which would change the bytes the camera/RTSP clients see. `rtpmap` goes through the typed `RtpMap` (its `Display` is `"{pt} {name}/{clock}[/{params}]"`, identical to the hand-built line).

**Files:**
- Modify: `transmux/Cargo.toml` (remove dev-dependency `sdp-types = "0.1"`; add `sdp-types = { version = "0.2", optional = true }` and `"dep:sdp-types"` to `std`)
- Modify: `transmux/src/rtp.rs` (ranges above), `transmux/src/lib.rs:337`
- Modify: `transmux/tests/rtp.rs:1182`, `:1500-1545`, `:1546` (the old `sdp_types` 0.1 call sites and the `&str` media-block argument), `transmux/tests/golden_wire.rs` (connection-address test)
- Test: `transmux/tests/golden_wire.rs` (Task 3 goldens, byte-for-byte) and `transmux/tests/rtp.rs` (new line-order test)

**Interfaces:**
```rust
// std only
pub struct RtpOutput { pub streams: Vec<RtpStream>, #[cfg(feature = "std")] pub sdp: String }
#[cfg(feature = "std")]
pub fn build_sdp_with_connection(connection_address: core::net::IpAddr, medias: Vec<sdp_types::Media>) -> String;
```
`sdp_types` 0.2 types now appear in transmux's public API (epoch rule: transmux is already major-class this wave). The default `std` build is unchanged for users; `--no-default-features` builds no longer have `RtpOutput::sdp`.

- [ ] **Step 1: Failing test — the golden stays byte-identical through the new builder**

Edit `transmux/tests/golden_wire.rs::connection_address_sdp_matches_golden` to the new signature:
```rust
    let media: Vec<sdp_types::Media> = sdp_types::Session::parse(MEDIA_BLOCK_SESSION.as_bytes())
        .expect("parse media block")
        .medias;
    let v4 = build_sdp_with_connection(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), media);
    let v6 = build_sdp_with_connection(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)), Vec::new());
```
with, next to `MEDIA_BLOCK`:
```rust
const MEDIA_BLOCK_SESSION: &str = concat!(
    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=x\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n",
    "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n",
    "a=fmtp:96 packetization-mode=1; profile-level-id=64000D; sprop-parameter-sets=Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA\r\n"
);
```
(a parseable session whose only role is to give the test a `Media`; `MEDIA_BLOCK` itself is deleted). The goldens `rtp-sdp-conn-v4.sdp`/`-v6.sdp`/`rtp-sdp-h264-aac.sdp` are NOT edited. Add to `tests/rtp.rs`:
```rust
/// RFC 8866 §5: v=, o=, s=, [c=], t=, then attributes, then each m= section.
/// An INDEPENDENT check: the writer is `sdp-types`, so its own parser can no
/// longer vouch for it.
#[test]
fn sdp_line_order_follows_rfc8866_section_5() {
    let out = packetise(&demux_fixture());
    let kinds: Vec<char> = out.sdp.split("\r\n").filter(|l| !l.is_empty()).map(|l| l.chars().next().unwrap()).collect();
    assert!(out.sdp.ends_with("\r\n") && !out.sdp.contains("\n\n"));
    assert_eq!(&kinds[..5], &['v', 'o', 's', 'c', 't']);
    let first_m = kinds.iter().position(|&c| c == 'm').unwrap();
    assert!(kinds[5..first_m].iter().all(|&c| c == 'a'), "{kinds:?}");
    assert!(out.sdp.lines().all(|l| l.len() >= 2 && l.as_bytes()[1] == b'='), "{}", out.sdp);
}
```
Update `tests/rtp.rs:1500-1545`: build the media vector from `sdp_types::Session::parse(out.sdp.as_bytes()).unwrap().medias` instead of filtering `m=`/`a=` lines into a `String`; the injection test becomes `build_sdp_with_connection(injected, Vec::new())`.

- [ ] **Step 2: Run — FAIL (does not compile: `Vec<Media>` vs `&str`)**

```bash
cargo test -p transmux --all-features --locked --test golden_wire --test rtp 2>&1 | grep -E '^error' | head -3
```

- [ ] **Step 3: Implement**

`Cargo.toml` per the dependency-add procedure (`sdp-types` 0.2.0 pulls `bstr`, `fallible-iterator`, `hex`, `thiserror`; expect the lock to hold sdp-types 0.1.8 and 0.2.0 side by side). `rtp.rs`:
```rust
#[cfg(feature = "std")]
use sdp_types::{Connection, Media, MediaType, Origin, RtpMap, Session, TransportProto};

#[cfg(feature = "std")]
fn build_sdp(medias: Vec<Media>) -> String {
    build_sdp_with_connection(LOCAL_CONNECTION_ADDRESS, medias)
}

/// … (keep the existing RFC 8866 §5.7 doc, now describing the `Vec<Media>` parameter) …
#[cfg(feature = "std")]
pub fn build_sdp_with_connection(connection_address: core::net::IpAddr, medias: Vec<Media>) -> String {
    let mut session = Session::new(
        Origin::with_ip_addr(0, 0, core::net::Ipv4Addr::LOCALHOST),
        "transmux RTP",
    );
    // `from_ip_addr` derives `IP4`/`IP6` from the address family, so no CR/LF or
    // space can reach the c= line (the typed address is the injection guard).
    session.connection = Some(Connection::from_ip_addr(connection_address));
    session.medias = medias;
    let mut out = Vec::new();
    // Writing into a `Vec<u8>` cannot fail; every field is a `String`.
    let _ = session.write(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(feature = "std")]
fn sdp_video(pt: u8, config: &crate::avc_config::AVCDecoderConfigurationRecord) -> Media {
    let profile_level_id = format!(
        "{:02X}{:02X}{:02X}",
        config.profile_indication, config.profile_compatibility, config.level_indication
    );
    let sprop = config
        .sps
        .iter()
        .map(|n| base64_encode(&n.0))
        .chain(config.pps.iter().map(|n| base64_encode(&n.0)))
        .collect::<Vec<_>>()
        .join(",");
    let mut media = Media::new(MediaType::Video, 0, TransportProto::RtpAvp, pt);
    media.add_attribute(RtpMap::new(pt, "H264", VIDEO_CLOCK_RATE));
    // fmtp parameter lists are codec payload-format logic (owner decision, spec §9.2).
    media.add_attribute_with_value(
        "fmtp",
        format!("{pt} packetization-mode=1; profile-level-id={profile_level_id}; sprop-parameter-sets={sprop}"),
    );
    media
}

#[cfg(feature = "std")]
fn sdp_audio(pt: u8, clock: u32, channels: u16, asc: &[u8]) -> Media {
    let config = hex::encode(asc);
    let mut media = Media::new(MediaType::Audio, 0, TransportProto::RtpAvp, pt);
    media.add_attribute(RtpMap::with_encoding_params(pt, "mpeg4-generic", clock, channels));
    media.add_attribute_with_value(
        "fmtp",
        format!(
            "{pt} streamtype=5; profile-level-id=1; mode=AAC-hbr; config={config}; \
             sizeLength={AAC_SIZE_LENGTH}; indexLength={AAC_INDEX_LENGTH}; \
             indexDeltaLength={AAC_INDEX_DELTA_LENGTH}"
        ),
    );
    media
}
```
In `package`: `#[cfg(feature = "std")] let mut sdp_media: Vec<Media> = Vec::new();`, the two push sites become `#[cfg(feature = "std")] sdp_media.push(sdp_video(pt, &config.config));` / `.push(sdp_audio(pt, clock, *channel_count, asc))` (the `asc_bytes(esds)?` call stays outside the cfg because it can fail), and the return builds `RtpOutput { streams, #[cfg(feature = "std")] sdp: build_sdp(sdp_media) }`. `RtpOutput`'s `sdp` field gets `#[cfg(feature = "std")]`. `lib.rs:337`: move `build_sdp_with_connection` out of the shared `pub use rtp::{…}` list into `#[cfg(feature = "std")] pub use rtp::build_sdp_with_connection;`. Add `#![cfg(feature = "std")]` to `tests/rtp.rs` only if the file has no feature gate and a no-default-features `cargo test --no-run` fails (CI runs `test --no-run --no-default-features --features std`).

- [ ] **Step 4: Run — PASS, byte-identical goldens, no_std build, consumers**

```bash
cargo test -p transmux --all-features --locked 2>&1 | grep -E 'test result|FAILED|panicked'
cargo build -p transmux --no-default-features --locked 2>&1 | tail -2
cargo test --no-run -p transmux --no-default-features --features std --locked 2>&1 | tail -2
cargo check -p multimux -p hls-runtime -p media-doctor -p rtsp-runtime --all-features --locked 2>&1 | grep -E '^error|Finished'
```

Expected: `golden_wire` 3 passed with NO golden edit (byte-identical SDP: `Session::write` emits `v, o, s, c, t, a…, m…` with `\r\n`, an empty `times` list writes `t=0 0`, `Origin::with_ip_addr(0, 0, 127.0.0.1)` writes `o=- 0 0 IN IP4 127.0.0.1`). If a byte differs, STOP: list the exact difference with an example in the CHANGELOG, update the golden in this commit, and confirm the rtsp/WebRTC interop suites (`cargo test -p rtsp-runtime -p webrtc-runtime --all-features --locked`) stay green — spec §10 permits semantic equality only with each difference listed.

- [ ] **Step 5: Commit**

```bash
git add transmux/Cargo.toml Cargo.lock transmux/src/rtp.rs transmux/src/lib.rs transmux/tests/rtp.rs transmux/tests/golden_wire.rs
git commit -m "refactor(transmux)!: RTP SDP built with sdp-types 0.2 Session::write; SDP generation is std-only"
```

---

### Task 15: Guards — lexical tripwire per crate (spec §5), the LAST code task

Same pattern as `transmux/tests/no_dom_guard.rs`: scan `src/**/*.rs` outside `#[cfg(test)]` items, with a reasoned allowlist, for the spec §5 patterns. Applies to all five crates of the cluster.

**Files:**
- Create: `broadcast-auth/tests/dehandroll_guard.rs`, then copy it to `transmux/tests/`, `timed-metadata/tests/`, `scte35-splice/tests/`, `broadcast-common/tests/` (identical file; only the allowlist may differ)

**Interfaces:** none (test-only).

- [ ] **Step 1: Write the guard**

```rust
//! Tripwire guard (spec §5, SP2–SP5): generic protocol and format work in this
//! crate goes through an established crate, not hand-rolled code. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` items and fails on:
//!
//! - an HTTP status/request line literal (`"HTTP/1.`) or a hand-framed
//!   `"\r\n\r\n"` header terminator;
//! - hand-split URLs: `find("://")`, `split_once("://")`,
//!   `strip_prefix("<scheme>://")`;
//! - hand-built SDP: a string literal starting `"v=0`, `"a=` or `"m=`;
//! - `civil_from_days` / `days_from_civil` (a hand-rolled calendar);
//! - a base64 alphabet literal;
//! - `thread::sleep` / `sleep(` in non-test code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper, a split
//! literal or a macro can evade it; code review is the real control. Every
//! `ALLOW` entry states why the hit is not hand-rolled protocol work, and a
//! stale entry (one that no longer matches) fails the test.

use std::fs;
use std::path::{Path, PathBuf};

/// `(needle, why it is banned)`. `strip_prefix("<scheme>://")` is checked
/// separately in `banned_in_line`.
const BANNED: &[(&str, &str)] = &[
    ("\"HTTP/1.", "hand-built HTTP status/request line"),
    ("\\r\\n\\r\\n", "hand-framed header terminator"),
    ("find(\"://\")", "hand-split URL"),
    ("split_once(\"://\")", "hand-split URL"),
    ("\"v=0", "hand-built SDP"),
    ("\"a=", "hand-built SDP attribute"),
    ("\"m=", "hand-built SDP media line"),
    ("civil_from_days", "hand-rolled calendar"),
    ("days_from_civil", "hand-rolled calendar"),
    ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", "base64 alphabet literal"),
    ("thread::sleep", "blocking sleep in non-test code"),
    ("sleep(", "sleep in non-test code"),
];

/// `(path suffix, needle, reason)` — reviewed exceptions. Empty by design.
const ALLOW: &[(&str, &str, &str)] = &[];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source with every `#[cfg(test)]` item (a `mod … { … }` block, or a single
/// line item) and every `//` comment line removed. Braces inside string
/// literals can fool the brace counter; that is within the tripwire's tolerance.
fn non_test_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate().peekable();
    while let Some((no, line)) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            let mut depth = 0i32;
            let mut opened = false;
            for (_, item) in lines.by_ref() {
                for c in item.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && item.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        if t.starts_with("//") {
            continue;
        }
        out.push((no + 1, line.to_string()));
    }
    out
}

fn banned_in_line(line: &str) -> Vec<(&'static str, &'static str)> {
    let mut hits: Vec<_> = BANNED
        .iter()
        .filter(|(needle, _)| line.contains(needle))
        .copied()
        .collect();
    if line.contains("strip_prefix(\"") && line.contains("://\")") {
        hits.push(("strip_prefix(\"<scheme>://\")", "hand-split URL"));
    }
    hits
}

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src_dir, &mut files);
    assert!(!files.is_empty());
    let mut violations = Vec::new();
    let mut used = vec![false; ALLOW.len()];
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        let rel = file.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().to_string_lossy().replace('\\', "/");
        for (no, line) in non_test_lines(&text) {
            for (needle, why) in banned_in_line(&line) {
                if let Some(i) = ALLOW.iter().position(|(suffix, n, _)| rel.ends_with(suffix) && *n == needle) {
                    used[i] = true;
                } else {
                    violations.push(format!("{rel}:{no}: {why}: {}", line.trim()));
                }
            }
        }
    }
    let stale: Vec<_> = ALLOW.iter().zip(&used).filter(|(_, u)| !**u).map(|(a, _)| a.0).collect();
    assert!(violations.is_empty(), "hand-rolled protocol code (use the established crate, or add a reasoned ALLOW entry):\n{}", violations.join("\n"));
    assert!(stale.is_empty(), "stale ALLOW entries: {stale:?}");
}

/// The scanner itself: it must bite on the shapes it claims to catch and
/// ignore test-only code, so the guard cannot silently stop guarding.
#[test]
fn scanner_catches_banned_shapes_and_skips_test_items() {
    let src = "fn a() { let _ = \"HTTP/1.1 200\"; }\n\
               #[cfg(test)]\nmod tests {\n    fn t() { let _ = \"HTTP/1.1 404\"; }\n}\n\
               fn b() { s.strip_prefix(\"rtsp://\"); }\n\
               // \"v=0 in a comment\n";
    let hits: Vec<_> = non_test_lines(src).into_iter().flat_map(|(_, l)| banned_in_line(&l)).collect();
    assert_eq!(hits.len(), 2, "{hits:?}");
}
```

- [ ] **Step 2: Copy to the other four crates and run each**

```bash
for c in transmux timed-metadata scte35-splice broadcast-common; do cp broadcast-auth/tests/dehandroll_guard.rs $c/tests/dehandroll_guard.rs; done
for c in broadcast-auth transmux timed-metadata scte35-splice broadcast-common; do
  echo "== $c"; cargo test -p $c --all-features --locked --test dehandroll_guard 2>&1 | grep -E 'test |test result|^[a-z/-]+\.rs:[0-9]+|:[0-9]+: '
done
```

Expected: both tests pass in all five crates (every hand-rolled site in the §3 inventory for this cluster was removed by Tasks 5–14). A violation line `path:line: reason: code` is either a missed inventory site (fix it in this task's commit and mention it in the report) or a reviewed exception: add ONE `ALLOW` entry with a one-line reason and list it in the report. One known candidate: `timed-metadata/src/webvtt/writer.rs:144` holds a `"…\r\n\r\n…"` cue-text literal — if it is outside a `#[cfg(test)]` item, allow it as `("webvtt/writer.rs", "\\r\\n\\r\\n", "WebVTT blank-line cue separator is the format's own syntax (spec §1 out-of-scope: product formats)")`.

- [ ] **Step 3: Prove the guard bites on real code (not just the scanner unit test)**

```bash
for c in broadcast-auth transmux timed-metadata scte35-splice broadcast-common; do
  printf '\nconst _GUARD_BITE: &str = "HTTP/1.1 200 OK";\n' >> $c/src/lib.rs
  cargo test -p $c --all-features --locked --test dehandroll_guard 2>&1 | grep -E 'FAILED|test result|_GUARD_BITE'
  git checkout -- $c/src/lib.rs
done
git status --short
```

Expected: five `FAILED` runs naming `src/lib.rs`; afterwards `git status --short` shows only the new guard files. Record the five FAIL lines in the report under "Revert-check evidence → guards".

- [ ] **Step 4: Commit**

```bash
git add broadcast-auth/tests/dehandroll_guard.rs transmux/tests/dehandroll_guard.rs timed-metadata/tests/dehandroll_guard.rs scte35-splice/tests/dehandroll_guard.rs broadcast-common/tests/dehandroll_guard.rs
git commit -m "test: dehandroll tripwire guards for transmux, timed-metadata, scte35-splice, broadcast-common, broadcast-auth"
```

---

### Task 16: CHANGELOGs, version notes, full gate. Do not merge.

**Files:**
- Modify: `broadcast-common/CHANGELOG.md`, `scte35-splice/CHANGELOG.md`, `timed-metadata/CHANGELOG.md`, `transmux/CHANGELOG.md`, `broadcast-auth/CHANGELOG.md` (each `## [Unreleased]`)
- Modify: `transmux/README.md`, `broadcast-auth/README.md`, crate-root `//!` docs and any `docs/` mention of the removed/changed APIs found by the greps below
- Modify: `.delegate/w1-t-report.md`

- [ ] **Step 1: CHANGELOG `[Unreleased]` entries (exact content; breaking ones marked)**

`broadcast-common/CHANGELOG.md`, `### Changed`:
- `hex::hex_encode` now delegates to the `hex` crate (`no_std` + `alloc`); output and signature unchanged. No API change.

`scte35-splice/CHANGELOG.md`, `### Changed`:
- `dvb_ta::base64_encode` delegates to the `base64` crate (`no_std` + `alloc`); output and signature unchanged. `base64` is now a normal dependency (was a dev-dependency).

`timed-metadata/CHANGELOG.md`:
- `### Added` — `anchor::try_format_rfc3339_ms`, `TimeAnchor::try_rfc3339` and `Error::TimestampOutOfRange(i64)` (new variant on the `#[non_exhaustive]` enum).
- `### Changed` — RFC 3339 formatting uses `jiff` (`no_std` + `alloc`); output is byte-identical for every instant in years -9999..=9999. `format_rfc3339_ms`/`TimeAnchor::rfc3339` now CLAMP an out-of-range instant instead of printing a many-digit year; `TimeAnchor::media_to_epoch_ms` saturates (was: sign-wrapping `u64 as i64` casts and a debug-build overflow panic); `convert::scte35_to_daterange` returns `Error::TimestampOutOfRange` for an unrepresentable `START-DATE`.
- `### Fixed` — `DateRange::parse_tag_line` no longer panics on a multi-byte character inside a `SCTE35-OUT/IN/CMD` hex value and no longer accepts a `+`/`-` sign as a hex digit (the hex now goes through the `hex` crate).

`transmux/CHANGELOG.md`:
- `### Changed (breaking)` —
  - `uri` module removed (`UriReference`, `resolve`, `merge`, `remove_dot_segments`, `resolve_segment`, `try_resolve*`, `first_forbidden_char` and the `RFC3986_*` tables, plus the crate-root `resolve_uri_reference`/`resolve_uri_segment`/`try_resolve_uri_reference`). Replaced by `base_url::{resolve, resolve_chain, first_forbidden_char}` over `url::Url` (`std` only; also `resolve_url_reference`/`resolve_base_url_chain` at the root). `Mpd::resolve_segment_url` now takes the MPD's own URL (`Option<&url::Url>`) as its first argument and returns `Option<String>`; `Mpd::try_resolve_segment_url` is removed. Differences from the old resolver, each with an example: a relative result that climbs above its base is an absolute path (`../x` with no base → `/x`, was `x`); non-ASCII is percent-encoded (`é.m4s` → `%C3%A9.m4s`); the `//g` network-path row serialises as `http://g/` and `http:g` against an `http` base resolves relatively (WHATWG, listed in `tests/base_url.rs`); a reference that does not parse is `None`.
  - `build_sdp_with_connection(IpAddr, Vec<sdp_types::Media>)` replaces the `&str` media-block parameter; RTP SDP generation is `std`-only (`RtpOutput::sdp` exists only with the `std` feature). The SDP bytes are identical (golden-tested). `sdp-types` 0.2 and `url` types appear in the public API.
  - DASH writer: `xs:duration` attributes are the shortest ISO 8601 form (`PT2.0S` → `PT2S`, `PT0.0S` → `PT0S`, `PT3.0S` → `PT3S`, `PT90.0S` → `PT1M30S`); equal durations, different bytes.
- `### Changed` — `jiff` replaces the hand-rolled `civil_from_days` (CLI `availabilityStartTime`) and the `xs:duration` parser (widening: `P1W` and a fractional hour/minute are accepted; calendar units, sign and lower-case are still rejected); `base64`/`hex` crates replace the hand-rolled codecs (`rtp::base64_decode` keeps its leniency: optional padding, trailing bits tolerated; a stray `=` inside the data is now an error; `rtp::hex_decode` reports a non-ASCII byte by code point).
- `### Fixed` — CLI `--key` no longer panics on a 32-byte non-ASCII argument and rejects a `+`/`-` sign in the hex.

`broadcast-auth/CHANGELOG.md`:
- `### Changed (breaking)` —
  - Signed-URL query is `application/x-www-form-urlencoded`: `kid` (defect 6) and `ip` are percent-encoded (`ip=2001:db8::1` → `ip=2001%3Adb8%3A%3A1`; verification percent-decodes, so a `+` in a URL minted by an older version now reads as a space). A bare `ip` key (no `=`) is rejected instead of ignored.
  - Digest `Authorization` is parsed with `http-auth`'s `ChallengeParser`: quoted-pairs are honoured, parameter names are case-insensitive, a repeated parameter or a second challenge is rejected, and a raw non-ASCII `username` is rejected (RFC 7616 §3.4: use `username*`/`userhash`).
  - Basic/Bearer use `headers::Authorization`: a Basic user-id containing `:` can no longer match (RFC 7617 §2); non-UTF-8 payloads are `Unauthorized`.
  - `Error::InvalidBearerToken` (new, `#[non_exhaustive]`): a Bearer token that cannot be a header value is refused instead of emitted.
  - `digest-uri` match requires the client's absolute-form `uri` in normalised spelling (`url::Url` round-trip) and an exact path+query match.
- `### Fixed` — `WWW-Authenticate` realm is rendered as an escaped quoted-string (a `"`, `\`, CR or LF in the realm can no longer break or split the header); Bearer header injection (above); `username` with `"`/`\` now verifies.
- `### Changed` — `lru` for the Digest nonce-count table, `hex` for nonces (no behaviour change).

- [ ] **Step 2: Docs sweep**

```bash
grep -rn 'transmux::uri\|resolve_uri\|try_resolve_segment_url\|UriReference\|hand-rolled base64\|build_sdp_with_connection' --include=*.md --include=*.rs transmux broadcast-auth timed-metadata scte35-splice broadcast-common docs README.md CLAUDE.md 2>/dev/null | grep -v 'CHANGELOG\|docs/superpowers' | head -20
```
Fix every hit (README coverage tables, crate-root docs, `transmux/src/lib.rs` module list, `CLAUDE.md` is the ORCHESTRATOR's in W3 — only report it). Also update the doc comments that still describe the removed internals: `transmux/src/lib.rs` (`uri` mention), `rtp.rs` module doc ("SDP … hand"), `broadcast-auth/src/lib.rs`/`server.rs` module docs (Digest parsing, nc table), `broadcast-auth/Cargo.toml` description if it names the parser.

- [ ] **Step 3: Full verification in this worktree**

```bash
cargo fmt --all
git diff --stat | tail -1
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" > target/gate-w1-t.log 2>&1
grep -E '^==|^rc=|GATE-DONE' target/gate-w1-t.log
grep -c '^rc=0' target/gate-w1-t.log
```

Expected: `GATE-DONE`, `14` lines `rc=0` (14/14). Any non-zero `rc` is fixed in this branch (commit the fix with the task it belongs to via `git commit --fixup`/a new `fix:` commit) and the gate re-run until 14/14. Then the cluster-specific extras the gate does not cover:

```bash
cargo test -p rtsp-runtime -p hls-runtime -p webrtc-runtime --all-features --locked 2>&1 | grep -E 'test result|FAILED' | sort | uniq -c
cargo +1.95.0 build -p transmux -p timed-metadata -p scte35-splice -p broadcast-common -p broadcast-auth --all-features --locked 2>&1 | tail -2
python3 tools/check-published-dep-consistency.py
```

Expected: consumers' suites unchanged vs. Task 0's multimux/rtsp/hls baseline; MSRV build `Finished`; the dep-consistency script clean (it is the gate's step 14; run alone for readable output).

- [ ] **Step 4: Version notes and report**

Write into `.delegate/w1-t-report.md` (the orchestrator copies to `.delegate/release-versions.txt`; do NOT edit Cargo.toml versions or that file):

| crate | current | change class | next | consumers that must move their `version =` requirement |
|---|---|---|---|---|
| broadcast-common | 9.4.0 | patch (new internal dependency, no API change) | 9.4.1 | none |
| scte35-splice | 2.1.0 | no change of its own (base64 dep); rides on its pending unreleased breaking entries | per those | none |
| timed-metadata | 0.5.0 | additive (+ fixes), not epoch-changing | 0.5.x | none (requirements `0.5` stay valid) |
| broadcast-auth | 0.3.1 | breaking (signed-URL wire, Digest strictness, `Error` variant) | 0.4.0 | rtsp-runtime, hls-runtime, multimux (`0.3` → `0.4`), `fuzz/Cargo.toml` is path-only |
| transmux | 0.24.2 | breaking (`uri` removed, `url`/`sdp-types` 0.2 in public API, std-only SDP, duration bytes) | 0.25.0 | hls-runtime, media-doctor, multimux, media-plane, rtmp-runtime dev-dep (`0.24` → `0.25`) — W1-R-low/W1-P/W2 own those edits |

Also record: the Task 0 baseline counts, the SP2.4 verification evidence, all revert-check evidence blocks (Tasks 6a, 6b, 9, 10a–10d, 11, 13, 15), the escalations E1–E3 below with their evidence, the `Cargo.lock` additions (`headers`, `headers-core`, `httpdate`, `mime`, `lru`, `jiff` + deps, `sdp-types 0.2.0` + deps), and the note that `Cargo.lock` will conflict with other W1 branches at merge.

- [ ] **Step 5: Commit and STOP**

```bash
git add broadcast-common/CHANGELOG.md scte35-splice/CHANGELOG.md timed-metadata/CHANGELOG.md transmux/CHANGELOG.md broadcast-auth/CHANGELOG.md
git add -u transmux/README.md broadcast-auth/README.md transmux/src/lib.rs
git add .delegate/w1-t-report.md
git commit -m "docs: W1-T CHANGELOG entries, version notes and report"
git log --oneline origin/main..HEAD
git status --short
```

**Do not merge, do not push, do not tag.** Hand `w1/t` (worktree `.worktree/w1-t`) and `.delegate/w1-t-report.md` back to the orchestrator for the adversarial review and the merge.

---

## Coverage table

Spec items owned by cluster T and the task that covers each.

| Spec item | Where | Task |
|---|---|---|
| SP2.4 first-task verification (ChallengeParser vs RFC 7616, ChallengeRef Display) | broadcast-auth | 1 |
| SP2.4 Authorization/Digest field parse via `http-auth` | `server.rs` `check_digest` | 10a |
| SP2.4 Authorization Basic/Bearer parse (spec fallback `headers`, scoped to token68 schemes) | `server.rs` `verify_basic/bearer` | 10c |
| SP2.4 WWW-Authenticate render | `server.rs` `render_challenge` | 10d (escaping fix) + **E1** (no crate can render) |
| SP2.4 `authenticator.rs` Bearer value | `authenticator.rs` | 10c |
| SP2.4 digest-uri match via `url` (§3 URL row + HTTP row) | `server.rs` `digest_uri_matches` | 10b |
| SP2.4 hex/unhex (§3 HTTP + hex rows) | `server.rs` `hex`/`unhex` | 8 |
| SP2.4 nonce table `lru` (§3 runtime row "BTreeMap LRU") | `server.rs` `NcTable` | 8 |
| SP2.4 signed URLs via `url`/`form_urlencoded`; **defect 6** `kid` unescaped | `signed_url.rs` | 9 (+ revert-check) |
| SP3 delete `uri.rs`; BaseURL via `Url::join`, option (a) synthetic base stripped by one tested function; MPD/file URL base | transmux | 13 |
| SP3 RFC 3986 §5.4 vectors become tests | `tests/base_url.rs` | 13 |
| SP3 `transmux uri.rs` CR/LF guard (§3 HTTP row) | `base_url::first_forbidden_char` | 13 (kept ahead of `join`, with evidence) |
| SP4 transmux `build_sdp*`/`sdp_video`/`sdp_audio` via sdp-types 0.2 `Session::write`; std-only; fmtp stays codec logic | `rtp.rs` | 14 |
| SP4 `rtp_sdp.rs` fmtp/rtpmap parse | `rtp_sdp.rs` | not changed — decision (§3 "which stays per decision", §9.2) |
| SP5 base64: transmux `rtp.rs` | `rtp.rs` | 7 |
| SP5 base64: scte35-splice `dvb_ta/stream_event.rs` | stream_event.rs | 7 |
| SP5 hex: transmux `rtp.rs`, `smooth_parse.rs`, `smooth.rs`, `dash.rs`, `sample_aes.rs`, `cli.rs` | transmux | 6a |
| SP5 hex: timed-metadata `daterange.rs` | timed-metadata | 6b |
| SP5 hex: broadcast-auth `server.rs` | broadcast-auth | 8 |
| SP5 hex: `broadcast_common::hex` keeps API, delegates (not breaking) | broadcast-common | 5 |
| SP5 dates: transmux `cli.rs` | `cli.rs` | 12a |
| SP5 dates: timed-metadata `anchor.rs` | `anchor.rs` | 11 |
| SP5 durations: transmux `dash_parse.rs` (parse) | `dash_parse.rs` | 12b |
| SP5 durations: transmux `dash.rs` (build) | `dash.rs` | 12c |
| SP5.4 tests: RFC 4648 vectors / RFC 3339 examples / XML Schema duration examples / MPD output byte-identical | tests | 7 / 11 / 12b / 12c (+ Task 3 goldens) |
| Goldens from main for every MPD/SDP/DATERANGE/signed-URL output touched | goldens | 2 (auth), 3 (SDP, IV; MPD/Smooth already committed), 4 (DATERANGE, RFC 3339, base64) |
| Guards (spec §5) for the five crates, in the last code task | `tests/dehandroll_guard.rs` | 15 |
| Found defects fixed with revert-checked tests: CLI `--key` panic, DATERANGE hex panic, Bearer header injection, realm injection, Digest quoted-pair/duplicate params, `format_rfc3339_ms`/`media_to_epoch_ms` range and overflow | various | 6a, 6b, 10c, 10d, 10a, 11 |
| §8 version notes; CHANGELOG with an example per intended difference; full 14-step gate | report | 16 |
| §3 sites NOT in cluster T (listed so no one hunts for them here) | hls-runtime dates/`..`, multimux dates/durations/hex, media-doctor, webrtc/rtsp/rtmp/srt | other waves (R-low, P, W2) |

## Escalations

Each item is something the plan could not do as the spec words it. None is dropped silently.

- **E1 — `WWW-Authenticate` cannot be rendered by `http-auth`'s `ChallengeRef` (spec §4 SP2.4, §10 "unverified").** Evidence: `~/.cargo/registry/src/*/http-auth-0.1.10/src/lib.rs` — `ChallengeRef` (`:117-151`) and `ParamValue` (`:678`) implement `Debug` only; `grep -c 'impl.*Display' lib.rs` = 0; no serializer exists in `parser.rs`/`digest.rs`/`basic.rs` (`digest.rs:535-570` has private client-side quoting helpers only). Verified by Task 1 step 2. Treatment in the plan: the renderer stays a small formatter (a documented exception to add to spec §9), but its real defect is fixed (quoted-string escaping, CR/LF dropped — Task 10d) and every rendering is parsed back through `ChallengeParser` in tests. Owner decision needed only if a rendering crate is preferred over the formatter: none exists (`headers` has no `WwwAuthenticate`).
- **E2 — Digest `username` is now ASCII-only (consequence of using `ChallengeParser`).** Evidence: `parser.rs` doc, "Doesn't allow non-ASCII characters"; pinned by `raw_non_ascii_username_is_rejected` (Task 10a) and `tests/http_auth_rfc7616.rs::raw_non_ascii_quoted_value_is_a_parse_error`. Browsers send raw UTF-8 usernames in Digest responses; the old hand parser accepted them. RFC 7616 §3.4 prescribes `username*`/`userhash` for non-ASCII, which this crate never supported. Decision for the owner: accept the (CHANGELOG-listed) behaviour change, or keep a lenient fallback for non-ASCII usernames — the plan implements the former because the spec fixes the parser choice (§4 SP2.4) and the fallback would re-introduce a hand parser.
- **E3 — Basic/Bearer cannot go through `ChallengeParser` (token68).** Evidence: `parser.rs` doc ("Doesn't allow `token68`"); pinned by `basic_token68_credentials_are_not_parseable` (Task 1). Treatment: `headers::Authorization<Basic|Bearer>` — the spec's own fallback, applied only to the two token68 schemes; Digest stays on `ChallengeParser`. Reported so the spec text (§4 SP2.4 "Authorization is parsed with http-auth") is amended to say Digest only.
- **E0 (conditional)** — only if Task 1 step 2 shows `ChallengeParser` failing on an RFC 7616 credential: the spec's Digest fallback applies (keep `split_digest_fields`, skip Task 10a, escalate the Digest gap). Not expected from the source reading.

Design notes that are not escalations but affect reviewers: (a) `timed-metadata`'s `format_rfc3339_ms`/`rfc3339` stay infallible and clamp, with fallible `try_*` twins, because a `Result` would force six workspace crates to take a caret-epoch change (spec §2 epoch purity; Task 11); (b) the independence of the `sdp-types` parse oracle in `tests/rtp.rs` ends when `sdp-types` becomes the writer, so Task 14 adds a hand-written RFC 8866 §5 line-order check and relies on the byte-for-byte golden; (c) two `sdp-types` versions (0.1.8 via rtsp-runtime, 0.2.0 via transmux) coexist in this branch's lock until W1-R-low merges.
