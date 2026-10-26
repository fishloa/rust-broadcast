#!/usr/bin/env python3
"""Generate the workspace crate index in docs/CRATES.md and README.md.

The machine-derivable part of the crate documentation -- which crates exist,
their in-tree versions, descriptions, publish flags, workspace dependencies,
dependency layers and whether CI proves them `no_std` -- is generated, so it
cannot drift from the code. Editorial text (domain, one-line summary, headline
standards, a short paragraph) lives in `docs/crates-index.toml`, one entry per
workspace member.

Sources, all read from the repository (no cargo, no network):

* `Cargo.toml` `[workspace] members`
* each member's `Cargo.toml`: version, description, publish, `[dependencies]`
  (normal dependencies only, including `[target.*.dependencies]`)
* `.github/workflows/ci.yml`: the crate list of the bare-metal `no_std` build
  loop, which is the only CI-verified `no_std` claim in the repo
* `docs/crates-index.toml`: the editorial fields

Generated regions are delimited by

    <!-- BEGIN GENERATED: <block> (tools/gen-crate-index.py) -->
    <!-- END GENERATED: <block> -->

and everything outside them is hand-written and left alone.

Usage:

    python3 tools/gen-crate-index.py            # rewrite the generated blocks
    python3 tools/gen-crate-index.py --check    # exit 1 if any block is stale
    python3 tools/gen-crate-index.py --verify-live
        # compare `crates_io` in docs/crates-index.toml with crates.io
        # (network; uses tools/crates-io.py; never run in CI)

`--check` also fails when a workspace member has no entry in
docs/crates-index.toml (or an entry names a crate that is not a member), so a
new crate cannot land unclassified.
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INDEX_TOML = ROOT / "docs" / "crates-index.toml"
CRATES_MD = ROOT / "docs" / "CRATES.md"
README_MD = ROOT / "README.md"
CI_YML = ROOT / ".github" / "workflows" / "ci.yml"

GENERATOR = "tools/gen-crate-index.py"
LIFECYCLES = ("active", "archived", "internal")
# Above this many dependents, "Used by" prints a count instead of the list.
USED_BY_LIST_MAX = 15


class IndexError_(Exception):
    """The inputs are inconsistent; the message says how to fix them."""


@dataclass
class Crate:
    name: str
    version: str
    description: str
    publish: bool
    has_lib: bool
    ws_deps: list[str]
    domain: str
    summary: str
    standards: str
    what: str
    crates_io: bool
    lifecycle: str
    no_std_ci: bool = False
    layer: int = 0
    dependents: list[str] = field(default_factory=list)


# --------------------------------------------------------------------- inputs


def _load_toml(path: Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def workspace_members(root: Path) -> list[str]:
    return list(_load_toml(root / "Cargo.toml")["workspace"]["members"])


def _inherit(value, ws_package: dict, key: str):
    if isinstance(value, dict) and value.get("workspace") is True:
        return ws_package.get(key)
    return value


def _dep_names(table: dict) -> list[str]:
    names = []
    for key, spec in table.items():
        if isinstance(spec, dict) and "package" in spec:
            names.append(spec["package"])
        else:
            names.append(key)
    return names


def ci_no_std_crates(root: Path) -> set[str]:
    """Crates built for `thumbv7em-none-eabi` in ci.yml's no-std job.

    Parsed as text (no PyYAML needed): the first `for c in ...; do` loop after
    the `no-std:` job key. Fails loudly if the shape changes, rather than
    silently reporting nothing as no_std.
    """
    text = (root / ".github" / "workflows" / "ci.yml").read_text()
    job = re.search(r"^  no-std:\n", text, re.M)
    if not job:
        raise IndexError_("ci.yml: no `no-std:` job found; update ci_no_std_crates()")
    loop = re.search(r"^\s*for c in ([^;\n]+); do$", text[job.end():], re.M)
    if not loop:
        raise IndexError_("ci.yml: no `for c in ...; do` loop in the no-std job")
    crates = set(loop.group(1).split())
    if len(crates) < 10:
        raise IndexError_(f"ci.yml: no-std loop parsed to only {len(crates)} crates")
    return crates


def load(root: Path = ROOT) -> tuple[list[dict], dict[str, Crate]]:
    ws = _load_toml(root / "Cargo.toml")["workspace"]
    ws_package = ws.get("package", {})
    members = list(ws["members"])
    member_names: dict[str, str] = {}
    manifests: dict[str, dict] = {}
    for path in members:
        manifest = _load_toml(root / path / "Cargo.toml")
        name = manifest["package"]["name"]
        member_names[name] = path
        manifests[name] = manifest

    index = _load_toml(root / "docs" / "crates-index.toml")
    domains = index.get("domain", [])
    domain_ids = [d["id"] for d in domains]
    entries = index.get("crate", {})

    missing = sorted(set(member_names) - set(entries))
    extra = sorted(set(entries) - set(member_names))
    problems = []
    if missing:
        problems.append(
            "workspace members with no [crate.<name>] entry in docs/crates-index.toml: "
            + ", ".join(missing)
        )
    if extra:
        problems.append(
            "docs/crates-index.toml entries that are not workspace members: " + ", ".join(extra)
        )

    no_std = ci_no_std_crates(root)
    unknown_no_std = sorted(no_std - set(member_names))
    if unknown_no_std:
        problems.append("ci.yml no-std loop names non-members: " + ", ".join(unknown_no_std))

    crates: dict[str, Crate] = {}
    for name, manifest in manifests.items():
        entry = entries.get(name)
        if entry is None:
            continue
        pkg = manifest["package"]
        deps: list[str] = _dep_names(manifest.get("dependencies", {}))
        for target in manifest.get("target", {}).values():
            deps += _dep_names(target.get("dependencies", {}))
        ws_deps = sorted({d for d in deps if d in member_names and d != name})
        publish = _inherit(pkg.get("publish", True), ws_package, "publish")
        publish = publish is not False and publish != []
        for key in ("domain", "summary", "standards", "what", "crates_io", "lifecycle"):
            if key not in entry:
                problems.append(f"[crate.{name}] is missing `{key}`")
        if problems and any(p.startswith(f"[crate.{name}]") for p in problems):
            continue
        if entry["domain"] not in domain_ids:
            problems.append(f"[crate.{name}] domain {entry['domain']!r} is not a [[domain]] id")
        if entry["lifecycle"] not in LIFECYCLES:
            problems.append(f"[crate.{name}] lifecycle {entry['lifecycle']!r} not in {LIFECYCLES}")
        if not publish and entry["crates_io"]:
            problems.append(f"[crate.{name}] crates_io = true but Cargo.toml says publish = false")
        if not publish and entry["lifecycle"] != "internal":
            problems.append(f"[crate.{name}] publish = false must be lifecycle = \"internal\"")
        crates[name] = Crate(
            name=name,
            version=str(_inherit(pkg.get("version"), ws_package, "version")),
            description=str(_inherit(pkg.get("description", ""), ws_package, "description")),
            publish=publish,
            has_lib=(root / member_names[name] / "src" / "lib.rs").exists(),
            ws_deps=ws_deps,
            domain=entry["domain"],
            summary=entry["summary"],
            standards=entry["standards"],
            what=entry["what"],
            crates_io=bool(entry["crates_io"]),
            lifecycle=entry["lifecycle"],
            no_std_ci=name in no_std,
        )
    if problems:
        raise IndexError_("\n".join(problems))

    _assign_layers(crates)
    for c in crates.values():
        for d in c.ws_deps:
            crates[d].dependents.append(c.name)
    for c in crates.values():
        c.dependents.sort()
    return domains, crates


def _assign_layers(crates: dict[str, Crate]) -> None:
    """layer = 0 with no workspace deps, else 1 + the deepest dependency."""
    done: dict[str, int] = {}

    def visit(name: str, stack: tuple[str, ...]) -> int:
        if name in done:
            return done[name]
        if name in stack:
            raise IndexError_("dependency cycle: " + " -> ".join(stack + (name,)))
        deps = crates[name].ws_deps
        layer = 0 if not deps else 1 + max(visit(d, stack + (name,)) for d in deps)
        done[name] = layer
        return layer

    for name in crates:
        crates[name].layer = visit(name, ())


def reduced_edges(crates: dict[str, Crate]) -> list[tuple[str, str]]:
    """Transitive reduction of the dependency graph (edge a -> b: a depends on b)."""
    reach: dict[str, set[str]] = {}

    def closure(name: str) -> set[str]:
        if name not in reach:
            out: set[str] = set()
            for d in crates[name].ws_deps:
                out.add(d)
                out |= closure(d)
            reach[name] = out
        return reach[name]

    edges = []
    for c in crates.values():
        for d in c.ws_deps:
            implied = any(d in closure(o) for o in c.ws_deps if o != d)
            if not implied:
                edges.append((c.name, d))
    return sorted(edges)


# ------------------------------------------------------------------ rendering


def _md(text: str) -> str:
    """Make a free-text field safe inside a Markdown table cell."""
    return text.replace("|", "\\|").replace("\n", " ")


def _anchor(name: str) -> str:
    return name.replace(".", "")


def _status(c: Crate) -> str:
    if c.lifecycle == "internal":
        return "internal, `publish = false`"
    if c.lifecycle == "archived":
        state = "archived"
        return f"{state}, on crates.io" if c.crates_io else f"{state}, never published"
    return "on crates.io" if c.crates_io else "not on crates.io"


def _badge(c: Crate) -> str:
    if not c.crates_io:
        return "—"
    return (
        f"[![crates.io](https://img.shields.io/crates/v/{c.name}.svg)]"
        f"(https://crates.io/crates/{c.name})"
    )


def _by_domain(domains: list[dict], crates: dict[str, Crate]) -> list[tuple[dict, list[Crate]]]:
    out = []
    for d in domains:
        members = sorted(
            (c for c in crates.values() if c.domain == d["id"]), key=lambda c: (c.layer, c.name)
        )
        if members:
            out.append((d, members))
    return out


def render_crates_md_block(domains: list[dict], crates: dict[str, Crate], root: Path = ROOT) -> str:
    lines: list[str] = []
    total = len(crates)
    published = sum(1 for c in crates.values() if c.crates_io)
    no_std = sum(1 for c in crates.values() if c.no_std_ci)
    lines.append(
        f"**{total} workspace members.** {published} are on crates.io; "
        f"{no_std} are proven `no_std` by CI (built with `--no-default-features` for "
        "`thumbv7em-none-eabi`). Versions below are the **in-tree** versions on this "
        "branch; the badge shows what crates.io currently serves."
    )
    lines.append("")

    # ---- overview table
    lines.append("### Overview")
    lines.append("")
    lines.append("| Crate | Domain | In-tree version | crates.io | `no_std` (CI) | Layer | Status |")
    lines.append("|---|---|---|---|---|---|---|")
    for d, members in _by_domain(domains, crates):
        for c in members:
            lines.append(
                f"| [`{c.name}`](#{_anchor(c.name)}) | {d['title']} | {c.version} | "
                f"{_badge(c)} | {'yes' if c.no_std_ci else 'no'} | {c.layer} | {_status(c)} |"
            )
    lines.append("")

    # ---- per-domain detail
    for d, members in _by_domain(domains, crates):
        lines.append(f"### {d['title']}")
        lines.append("")
        lines.append(d["blurb"])
        lines.append("")
        for c in members:
            lines.append(f"#### {c.name}")
            lines.append("")
            lines.append(c.what)
            lines.append("")
            lines.append(f"- **Standards:** {c.standards}")
            lines.append(f"- **Cargo description:** {c.description}")
            lines.append(
                f"- **Version (in-tree):** {c.version} · **Status:** {_status(c)} · "
                f"**`no_std` proven in CI:** {'yes' if c.no_std_ci else 'no'} · "
                f"**Layer:** {c.layer}"
            )
            deps = ", ".join(f"[`{x}`](#{_anchor(x)})" for x in c.ws_deps) or "none"
            if len(c.dependents) > USED_BY_LIST_MAX:
                users = f"{len(c.dependents)} workspace crates"
            else:
                users = ", ".join(f"[`{x}`](#{_anchor(x)})" for x in c.dependents) or "none"
            lines.append(f"- **Depends on (workspace):** {deps}")
            lines.append(f"- **Used by (workspace):** {users}")
            links = [f"[README](../{c.name}/README.md)"] if (root / c.name / "README.md").exists() else []
            if (root / c.name / "CHANGELOG.md").exists():
                links.append(f"[CHANGELOG](../{c.name}/CHANGELOG.md)")
            if (root / c.name / "docs").is_dir():
                links.append(f"[docs + spec pointer](../{c.name}/docs/)")
            if c.crates_io:
                links.append(f"[crates.io](https://crates.io/crates/{c.name})")
                if c.has_lib:
                    links.append(f"[docs.rs](https://docs.rs/{c.name})")
            if links:
                lines.append("- **Links:** " + " · ".join(links))
            lines.append("")

    # ---- layers
    lines.append("### Dependency layers")
    lines.append("")
    lines.append(
        "Layer 0 has no workspace dependencies; every other crate sits one layer above "
        "its deepest workspace dependency. Normal dependencies only (dev-dependencies "
        "are excluded, so the test helpers appear at layer 0 with no edges)."
    )
    lines.append("")
    max_layer = max(c.layer for c in crates.values())
    lines.append("```text")
    for layer in range(max_layer + 1):
        names = sorted(c.name for c in crates.values() if c.layer == layer)
        lines.append(f"layer {layer}: " + ", ".join(names))
    lines.append("```")
    lines.append("")
    lines.append(
        "The same graph as a diagram (transitive edges removed, so an arrow means "
        "\"depends on directly, and not already through another arrow\"; the internal "
        "test helpers are omitted):"
    )
    lines.append("")
    lines.append("```mermaid")
    lines.append("flowchart BT")
    for layer in range(max_layer + 1):
        names = sorted(
            c.name for c in crates.values() if c.layer == layer and c.lifecycle != "internal"
        )
        if not names:
            continue
        lines.append(f'  subgraph L{layer}["layer {layer}"]')
        for n in names:
            lines.append(f'    {_mermaid_id(n)}["{n}"]')
        lines.append("  end")
    for a, b in reduced_edges(crates):
        lines.append(f"  {_mermaid_id(a)} --> {_mermaid_id(b)}")
    lines.append("```")
    return "\n".join(lines)


def _mermaid_id(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9_]", "_", name)


def render_readme_block(domains: list[dict], crates: dict[str, Crate]) -> str:
    lines: list[str] = []
    for d, members in _by_domain(domains, crates):
        if d["id"] == "internal":
            continue
        lines.append(f"**{d['title']}**")
        lines.append("")
        lines.append("| Crate | What it does | Headline standards | Status |")
        lines.append("|---|---|---|---|")
        for c in sorted(members, key=lambda c: c.name):
            badge = _badge(c) if c.crates_io else _status(c)
            if c.lifecycle == "archived" and c.crates_io:
                badge = f"{badge} archived"
            lines.append(
                f"| [`{c.name}`]({c.name}/) | {_md(c.summary)} | {_md(c.standards)} | {badge} |"
            )
        lines.append("")
    internal = sorted(c.name for c in crates.values() if c.lifecycle == "internal")
    lines.append(
        "Plus "
        + ", ".join(f"`{n}`" for n in internal)
        + ": `publish = false` test helpers, see [docs/CRATES.md](docs/CRATES.md)."
    )
    return "\n".join(lines)


# ------------------------------------------------------------ block splicing


def _markers(block: str) -> tuple[str, str]:
    return (
        f"<!-- BEGIN GENERATED: {block} ({GENERATOR}) -->",
        f"<!-- END GENERATED: {block} -->",
    )


def splice(text: str, block: str, body: str, path: Path) -> str:
    begin, end = _markers(block)
    pattern = re.compile(re.escape(begin) + r"\n.*?" + re.escape(end), re.S)
    if len(pattern.findall(text)) != 1:
        raise IndexError_(f"{path}: expected exactly one `{begin}` ... `{end}` region")
    return pattern.sub(lambda _: f"{begin}\n{body}\n{end}", text)


def expected_files(root: Path = ROOT) -> dict[Path, str]:
    domains, crates = load(root)
    out = {}
    crates_md = root / "docs" / "CRATES.md"
    out[crates_md] = splice(
        crates_md.read_text(), "crate-index", render_crates_md_block(domains, crates, root), crates_md
    )
    readme = root / "README.md"
    out[readme] = splice(
        readme.read_text(), "crate-table", render_readme_block(domains, crates), readme
    )
    return out


# ----------------------------------------------------------------- live check


def verify_live(root: Path = ROOT) -> int:
    spec = importlib.util.spec_from_file_location("crates_io", root / "tools" / "crates-io.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    _, crates = load(root)
    bad = 0
    for c in sorted(crates.values(), key=lambda c: c.name):
        if not c.publish:
            continue
        try:
            live = mod.live_version(c.name)
        except Exception as exc:  # noqa: BLE001 - report, never guess
            print(f"{c.name}: lookup failed ({exc}); status unknown")
            bad += 1
            continue
        on = live is not None
        note = f"live {live}, in-tree {c.version}" if on else "not on crates.io"
        if on != c.crates_io:
            print(f"{c.name}: docs/crates-index.toml says crates_io = {c.crates_io}, but {note}")
            bad += 1
        else:
            print(f"{c.name}: ok ({note})")
        time.sleep(1.1)  # crates.io rate limit: one request per second
    return 1 if bad else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true", help="exit 1 if a generated block is stale")
    ap.add_argument(
        "--verify-live", action="store_true", help="compare crates_io flags with crates.io"
    )
    args = ap.parse_args(argv)
    try:
        if args.verify_live:
            return verify_live()
        files = expected_files()
    except IndexError_ as exc:
        print(f"gen-crate-index: {exc}", file=sys.stderr)
        return 1
    stale = [p for p, text in files.items() if p.read_text() != text]
    if args.check:
        if stale:
            for p in stale:
                print(f"stale: {p.relative_to(ROOT)}", file=sys.stderr)
            print(f"run `python3 {GENERATOR}` and commit the result", file=sys.stderr)
            return 1
        print("crate index up to date")
        return 0
    for p in stale:
        p.write_text(files[p])
        print(f"updated {p.relative_to(ROOT)}")
    if not stale:
        print("crate index already up to date")
    return 0


if __name__ == "__main__":
    sys.exit(main())
