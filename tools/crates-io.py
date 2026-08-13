#!/usr/bin/env python3
"""Query crates.io for published versions — the ONLY supported way to do it.

Usage:
    tools/crates-io.py version <crate> [<crate> ...]
    tools/crates-io.py check   <crate>=<expected-version> [...]

`version` prints one line per crate: `<crate> <live-version|ABSENT>`.
`check` verifies each crate is live at exactly the given version; exit 1 if not.

# Why this file exists

crates.io answers a request with **no `User-Agent` header** with **HTTP 403**.
Hand-rolled `curl https://crates.io/api/v1/crates/<name>` therefore returns an
error body, and code that parses it "leniently" reports the crate as *not
published* — turning a rejected request into a confident false negative.

That happened for real in this repo: a release audit concluded `playout-runtime`
and `ssai-runtime` were absent from crates.io and that depending on them would
block publishing `multimux`. Both were live at 0.1.0 the whole time. The wrong
conclusion reached a commit message, a design doc and a GitHub issue before a
contradicting lookup exposed it.

So this script enforces two rules that a hand-rolled call keeps getting wrong:

1. **Always send a `User-Agent`.** crates.io requires one.
2. **Never turn a failed request into a negative answer.** Only a genuine
   HTTP 404 means "not published". Any other non-200, a timeout, or an
   unparseable body is an ERROR and exits non-zero — it never prints ABSENT.

If you are about to write `curl .../crates.io/...` anywhere, use this instead.
"""

from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request

API = "https://crates.io/api/v1/crates/{}"
USER_AGENT = "rust-broadcast-release-tooling (https://github.com/fishloa/rust-broadcast)"
TIMEOUT_SECS = 20


class LookupError_(Exception):
    """A lookup that did not produce a trustworthy answer."""


def live_version(crate: str) -> str | None:
    """Return the max published version, or None if the crate genuinely is not
    published (HTTP 404).

    Raises `LookupError_` for anything else — a 403, a 5xx, a timeout, a body
    that will not parse. Those mean *unknown*, never *absent*.
    """
    req = urllib.request.Request(API.format(crate), headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SECS) as resp:
            if resp.status != 200:
                raise LookupError_(f"{crate}: HTTP {resp.status} (not an answer)")
            body = json.load(resp)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None  # the one case that genuinely means "not published"
        if e.code == 403:
            raise LookupError_(
                f"{crate}: HTTP 403 — crates.io rejected the request. This is "
                f"almost always a missing User-Agent. It does NOT mean the "
                f"crate is unpublished."
            ) from e
        raise LookupError_(f"{crate}: HTTP {e.code}") from e
    except Exception as e:  # timeout, DNS, bad JSON, ...
        raise LookupError_(f"{crate}: lookup failed ({e.__class__.__name__}: {e})") from e

    try:
        return body["crate"]["max_version"]
    except (KeyError, TypeError) as e:
        raise LookupError_(f"{crate}: unexpected response shape") from e


def cmd_version(crates: list[str]) -> int:
    rc = 0
    for c in crates:
        try:
            v = live_version(c)
        except LookupError_ as e:
            print(f"ERROR {e}", file=sys.stderr)
            rc = 2
            continue
        print(f"{c} {v if v is not None else 'ABSENT'}")
    return rc


def cmd_check(pairs: list[str]) -> int:
    rc = 0
    for pair in pairs:
        if "=" not in pair:
            print(f"ERROR bad argument {pair!r}, expected <crate>=<version>", file=sys.stderr)
            rc = 2
            continue
        crate, expected = pair.split("=", 1)
        try:
            v = live_version(crate)
        except LookupError_ as e:
            print(f"ERROR {e}", file=sys.stderr)
            rc = 2
            continue
        if v == expected:
            print(f"ok   {crate} {expected} is live")
        elif v is None:
            print(f"FAIL {crate} is not published (expected {expected})")
            rc = 1
        else:
            print(f"FAIL {crate} live={v} expected={expected}")
            rc = 1
    return rc


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    cmd, args = argv[1], argv[2:]
    if cmd == "version":
        return cmd_version(args)
    if cmd == "check":
        return cmd_check(args)
    print(f"unknown command {cmd!r}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
