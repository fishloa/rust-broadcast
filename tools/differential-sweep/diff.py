#!/usr/bin/env python3
"""Compare the JSONL outputs of `sweep` built against the baseline and HEAD.

    diff.py BASELINE.jsonl HEAD.jsonl [--expected expected.json] [--top N]
            [--json OUT.json]

Every JSONL line is `{"fixture", "probe", "result"}`. Results are flattened to
dotted leaf paths and compared. Each difference is classified:

  INTENDED     matches an entry of `expected.json` (an acknowledged diff with a
               written reason; cite the CHANGELOG BREAKING line / spec page)
  BUG          matches an `expected.json` entry whose class is "BUG" (triaged as
               a defect, not yet fixed)
  UNEXPLAINED  everything else

Exit status is non-zero when any BUG or UNEXPLAINED difference remains, so the
sweep can gate a release. Keys starting with `head_only_` exist in only one tree
by construction and are ignored.

`expected.json` is a list of objects:
  {"probe": "<glob>", "fixture": "<glob, default *>", "path": "<glob, default *>",
   "class": "INTENDED" (default) | "BUG", "reason": "<required>"}
Globs use fnmatch; `path` is matched against the dotted path with numeric
segments kept (use `*`).
"""

import argparse
import fnmatch
import json
import re
import sys
from collections import defaultdict

IGNORE_PREFIX = "head_only_"
MISSING = "<absent>"


def load(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            out[(rec["fixture"], rec["probe"])] = rec["result"]
    return out


def flatten(value, prefix=""):
    """Yield (dotted_path, leaf) pairs. Lists are indexed; empty containers are
    leaves so an emptied list still differs from a populated one."""
    if isinstance(value, dict):
        if not value:
            yield prefix, {}
            return
        for k in sorted(value):
            if k.startswith(IGNORE_PREFIX):
                continue
            yield from flatten(value[k], f"{prefix}.{k}" if prefix else k)
    elif isinstance(value, list):
        if not value:
            yield prefix, []
            return
        for i, v in enumerate(value):
            yield from flatten(v, f"{prefix}[{i}]")
    else:
        yield prefix, value


def diff_records(base, head):
    """Return a list of dicts {fixture, probe, path, base, head}."""
    rows = []
    for key in sorted(set(base) | set(head)):
        fixture, probe = key
        if key not in head:
            rows.append(dict(fixture=fixture, probe=probe, path="<record>",
                             base="present", head=MISSING))
            continue
        if key not in base:
            rows.append(dict(fixture=fixture, probe=probe, path="<record>",
                             base=MISSING, head="present"))
            continue
        fb = dict(flatten(base[key]))
        fh = dict(flatten(head[key]))
        for path in sorted(set(fb) | set(fh)):
            b, h = fb.get(path, MISSING), fh.get(path, MISSING)
            if b != h:
                rows.append(dict(fixture=fixture, probe=probe, path=path, base=b, head=h))
    return rows


def classify(rows, expected):
    for r in rows:
        r["class"] = "UNEXPLAINED"
        r["reason"] = ""
        for e in expected:
            if not fnmatch.fnmatchcase(r["probe"], e.get("probe", "*")):
                continue
            if not fnmatch.fnmatchcase(r["fixture"], e.get("fixture", "*")):
                continue
            if not fnmatch.fnmatchcase(r["path"], e.get("path", "*")):
                continue
            r["class"] = e.get("class", "INTENDED")
            r["reason"] = e["reason"]
            break
    return rows


def group_key(row):
    return (row["probe"], re.sub(r"\d+", "#", row["path"]))


def fmt(v, width=48):
    s = json.dumps(v) if not isinstance(v, str) else v
    return s if len(s) <= width else s[: width - 3] + "..."


def render(rows, top):
    groups = defaultdict(list)
    for r in rows:
        groups[group_key(r)].append(r)
    ranked = sorted(
        groups.items(),
        key=lambda kv: (
            0 if any(r["class"] != "INTENDED" for r in kv[1]) else 1,
            -len({r["fixture"] for r in kv[1]}),
            kv[0],
        ),
    )
    lines = [f"{'class':<11} {'fixtures':>8} {'rows':>6}  probe / path  (example: base -> head)"]
    for (probe, path), rs in ranked[:top]:
        classes = sorted({r["class"] for r in rs})
        ex = rs[0]
        lines.append(
            f"{'/'.join(classes):<11} {len({r['fixture'] for r in rs}):>8} {len(rs):>6}  "
            f"{probe} {path}\n{'':>29}e.g. {ex['fixture']}: {fmt(ex['base'])} -> {fmt(ex['head'])}"
        )
    if len(ranked) > top:
        lines.append(f"... {len(ranked) - top} more groups (use --top / --json)")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("baseline")
    ap.add_argument("head")
    ap.add_argument("--expected", help="expected.json of acknowledged diffs")
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--json", help="write every classified row to this file")
    args = ap.parse_args(argv)

    expected = []
    if args.expected:
        with open(args.expected, encoding="utf-8") as f:
            expected = json.load(f)
        for e in expected:
            if not e.get("reason"):
                ap.error(f"expected.json entry without a reason: {e}")

    base, head = load(args.baseline), load(args.head)
    rows = classify(diff_records(base, head), expected)

    counts = defaultdict(int)
    for r in rows:
        counts[r["class"]] += 1
    fixtures = {k[0] for k in set(base) | set(head)}
    print(f"records: baseline={len(base)} head={len(head)} fixtures={len(fixtures)}")
    print(
        "differences: "
        + ", ".join(f"{c}={counts.get(c, 0)}" for c in ("UNEXPLAINED", "BUG", "INTENDED"))
        + f" (rows={len(rows)}, differing records="
        f"{len({(r['fixture'], r['probe']) for r in rows})})"
    )
    if rows:
        print(render(rows, args.top))
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(rows, f, indent=1)
    return 1 if counts.get("UNEXPLAINED") or counts.get("BUG") else 0


if __name__ == "__main__":
    sys.exit(main())
