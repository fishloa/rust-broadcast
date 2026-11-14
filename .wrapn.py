import glob
import re
import sys

METHODS = [
    "on_mpd_with", "on_mpd_failure", "on_mpd", "poll", "on_init", "on_index",
    "on_segment", "retry", "skip", "retry_mpd", "utc_at",
]

ALLOWED = re.compile(r"^[0-9A-Za-z_.*&+ ]+$|^[A-Za-z_][A-Za-z0-9_.]*\([^()]*\)$")


def split_top(inner):
    parts = []
    depth = 0
    cur = ""
    instr = None
    prev = ""
    for ch in inner:
        if instr:
            cur += ch
            if ch == instr and prev != "\\":
                instr = None
            prev = ch
            continue
        if ch in "\"'":
            instr = ch
            cur += ch
            prev = ch
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
        prev = ch
    parts.append(cur)
    return parts


def wrap(paths, helper):
    trigger = re.compile(r"\.(" + "|".join(METHODS) + r")\(")
    for f in paths:
        s = open(f).read()
        out = []
        i = 0
        n = 0
        while True:
            m = trigger.search(s, i)
            if not m:
                out.append(s[i:])
                break
            j = m.end()
            depth = 1
            start = j
            instr = None
            prev = ""
            while j < len(s) and depth > 0:
                ch = s[j]
                if instr:
                    if ch == instr and prev != "\\":
                        instr = None
                elif ch in "\"'":
                    instr = ch
                elif ch in "([{":
                    depth += 1
                elif ch in ")]}":
                    depth -= 1
                prev = ch
                j += 1
            if depth != 0:
                out.append(s[i:])
                break
            inner = s[start : j - 1]
            parts = split_top(inner)
            out.append(s[i : m.start()])
            if not parts:
                out.append(m.group(0) + inner + ")")
                i = j
                continue
            a = parts[0].strip()
            if a and not a.startswith("now(") and ALLOWED.match(a) and a not in ("None",):
                parts[0] = " " + helper + "(" + a + ")"
                out.append("." + m.group(1) + "(" + ",".join(parts) + ")")
                n += 1
            else:
                out.append(m.group(0) + inner + ")")
            i = j
        ns = "".join(out)
        if n:
            open(f, "w").write(ns)
            print(n, f)


if __name__ == "__main__":
    wrap(sys.argv[2:], sys.argv[1])
