#!/usr/bin/env python3
"""Generate dvb-si's coverage documentation from the crate's single sources of truth.

Outputs (all deterministic — same inputs, byte-identical output):

* ``dvb-si/docs/COVERAGE.md`` — the full coverage reference: every table_id,
  descriptor tag, descriptor_tag_extension, MPEG-2 extension tag, AIT / INT /
  UNT / DII / RNT scoped descriptor, private (PDS-scoped) descriptor, DSM-CC
  carousel message, Annex A text charset, plus the honest "Not implemented"
  rows and a citation cross-check.
* the ``<!-- BEGIN GENERATED: dvb-si-at-a-glance -->`` block in
  ``dvb-si/README.md`` (counts + the standards table), and
* the ``<!-- BEGIN GENERATED: dvb-si-at-a-glance -->`` block in the crate-root
  ``//!`` docs of ``dvb-si/src/lib.rs``, and
* the ``<!-- BEGIN GENERATED: dvb-si-docs-index -->`` block in
  ``dvb-si/docs/README.md`` (the spec-transcription index).

Inputs ("what the code implements" — read lexically, never hand-typed):

* ``declare_tables!``            dvb-si/src/tables/any.rs
* ``declare_descriptors!``       dvb-si/src/descriptors/any.rs
* ``declare_extension_bodies!``  dvb-si/src/descriptors/extension/mod.rs
* ``declare_ait_descriptors!``   dvb-si/src/descriptors/ait/mod.rs
* ``Mpeg2ExtensionBody``         dvb-si/src/descriptors/mpeg2_extension.rs
* each module's leading ``//!`` paragraph (the spec citation), ``TAG`` consts,
  the ``DescriptorRegistry::with_*`` opt-ins, the ``Charset`` enum, and the
  vendored TSDuck registries (``dvb-si/registries/*.names``).

and ("what the standards define"): ``dvb-si/docs/coverage/allocations.tsv``,
the hand-transcribed allocation lists every "Not implemented" row comes from.

Usage::

    python3 tools/gen-dvbsi-coverage.py           # rewrite the outputs
    python3 tools/gen-dvbsi-coverage.py --check   # exit 1 (with a diff) if stale

``--check`` is what ``dvb-si/tests/docs_coverage_drift.rs`` runs, so CI fails
whenever a table/descriptor is added, removed or re-cited without
regenerating the docs.
"""

from __future__ import annotations

import difflib
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATE = ROOT / "dvb-si"
SRC = CRATE / "src"
COVERAGE = CRATE / "docs" / "COVERAGE.md"
README = CRATE / "README.md"
LIB = SRC / "lib.rs"
ALLOCATIONS = CRATE / "docs" / "coverage" / "allocations.tsv"

BEGIN = "<!-- BEGIN GENERATED: dvb-si-at-a-glance (tools/gen-dvbsi-coverage.py) -->"
END = "<!-- END GENERATED: dvb-si-at-a-glance -->"

REPO_BLOB = "https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/"

# Tracking issues the generated docs link to.
ISSUES = {
    "namespace": 1285,  # INT/UNT/RNT loops decoded in the SI/MPEG tag namespace
    "citations": 1287,  # module docs citing the wrong clause number
}


class GenError(Exception):
    """A source of truth could not be read the way this generator expects."""


# ───────────────────────────── helpers ──────────────────────────────


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as e:  # pragma: no cover - environment problem
        raise GenError(f"cannot read {path}: {e}") from e


def rel(path: Path) -> str:
    return path.relative_to(CRATE).as_posix()


def macro_body(text: str, name: str, path: Path) -> str:
    """Body of the ``name! { ... }`` invocation (not the macro_rules definition)."""
    m = re.search(r"^" + re.escape(name) + r"!\s*\{", text, re.M)
    if not m:
        raise GenError(f"{rel(path)}: no `{name}! {{` invocation found")
    depth = 0
    i = m.end() - 1
    for j in range(i, len(text)):
        c = text[j]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[i + 1 : j]
    raise GenError(f"{rel(path)}: unbalanced `{name}!` invocation")


def strip_line_comments(body: str) -> str:
    return "\n".join(re.sub(r"//.*$", "", line) for line in body.splitlines())


def module_doc(path: Path) -> str:
    """First paragraph of the leading ``//!`` block, flattened to one line."""
    lines = []
    for line in read(path).splitlines():
        s = line.strip()
        if not s.startswith("//!"):
            if lines or s == "" or s.startswith("//"):
                if lines:
                    break
                continue
            break
        t = s[3:].strip()
        if t == "":
            if lines:
                break
            continue
        lines.append(t)
    doc = " ".join(lines)
    # [`x`](link) / [x](link) / [`x`] -> x ; drop backticks.
    doc = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", doc)
    doc = re.sub(r"\[([^\]]+)\]", r"\1", doc)
    doc = doc.replace("`", "")
    return doc.strip()


_ABBREV = ("Rec", "No", "p", "pp", "Vol", "e.g", "i.e", "cf", "Fig")


def citation(doc: str) -> str:
    """The spec-citation part of a module doc paragraph (after the em dash)."""
    if " — " in doc:
        doc = doc.split(" — ", 1)[1]
    # Cut at the first real sentence end (not "Rec. ITU-T", "p. 54", ...).
    for m in re.finditer(r"\.(\s+|$)", doc):
        before = doc[: m.start()]
        word = re.split(r"[\s(]", before)[-1] if before else ""
        if word in _ABBREV or re.fullmatch(r"[A-Z]", word):
            continue
        doc = before
        break
    doc = doc.rstrip(" .;:,")
    if len(doc) > 150:
        cut = doc[:150].rsplit(" ", 1)[0]
        doc = cut.rstrip(" ,;(") + " …"
    return doc.replace("|", "\\|")


def cited_clauses(cite: str) -> set[str]:
    """Clause numbers a citation names explicitly (``§x.y``, ``clause x.y``, ``Annex X``).

    A range (``clauses 2.6.106–2.6.118``) is expanded over its last component.
    """
    out = set()
    for m in re.finditer(
        r"(?:§\s?|clauses?\s+|Annex\s+)([A-Z](?:\.\d+)*|\d+(?:\.\d+)+)(?:\s*[–-]\s*(\d+(?:\.\d+)+))?", cite
    ):
        out.add(m.group(1))
        if m.group(2):
            a, b = m.group(1).split("."), m.group(2).split(".")
            if len(a) == len(b) and a[:-1] == b[:-1] and a[-1].isdigit() and b[-1].isdigit():
                for n in range(int(a[-1]), int(b[-1]) + 1):
                    out.add(".".join(a[:-1] + [str(n)]))
    return out


def clause_agrees(verified: str, named: set[str]) -> bool:
    """A cited clause agrees when equal, a parent of, or a sub-clause of the verified one."""
    return any(
        verified == c or verified.startswith(c + ".") or c.startswith(verified + ".") for c in named
    )


def find_type_file(type_name: str, root: Path) -> Path:
    pat = re.compile(r"^pub (?:struct|enum) " + re.escape(type_name) + r"\b", re.M)
    hits = sorted(p for p in root.rglob("*.rs") if pat.search(read(p)))
    if not hits:
        raise GenError(f"type `{type_name}` not found under {rel(root)}")
    return hits[0]


def parse_code(code: str) -> tuple[int, int]:
    code = code.strip()
    if "-" in code:
        a, b = code.split("-")
        return int(a, 16), int(b, 16)
    v = int(code, 16)
    return v, v


def fmt_code(lo: int, hi: int) -> str:
    return f"0x{lo:02X}" if lo == hi else f"0x{lo:02X}–0x{hi:02X}"


def md_link(path: Path, text: str | None = None) -> str:
    r = rel(path)
    return f"[`{text or r}`](../{r})"


# ──────────────────────── sources of truth ─────────────────────────


@dataclass
class Impl:
    """One implemented wire structure."""

    namespace: str
    ranges: list[tuple[int, int]]
    variant: str
    rust_type: str
    module: Path
    dispatched: bool = True
    status: str = "Full"
    note: str = ""
    cite: str = ""
    extra: dict = field(default_factory=dict)

    def covers(self, lo: int, hi: int) -> bool:
        return any(a <= lo and hi <= b for a, b in self.ranges)


@dataclass
class Alloc:
    namespace: str
    lo: int
    hi: int
    name: str
    standard: str
    clause: str
    # PDF page (1-based, of the vendored PDF of the edition in `standard`) on which the
    # clause heading AND its syntax were read from the rendered page. Empty = unverified.
    verified_page: str = ""


def load_allocations() -> list[Alloc]:
    out = []
    for n, line in enumerate(read(ALLOCATIONS).splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        cols = line.rstrip("\n").split("\t")
        while len(cols) < 6:  # empty trailing `clause` / `verified_page` columns
            cols.append("")
        if len(cols) != 6:
            raise GenError(f"{rel(ALLOCATIONS)}:{n}: expected at most 6 TAB-separated columns, got {len(cols)}")
        ns, code, name, std, clause, page = (c.strip() for c in cols)
        if page and not page.isdigit():
            raise GenError(f"{rel(ALLOCATIONS)}:{n}: verified_page must be a PDF page number, got {page!r}")
        lo, hi = parse_code(code)
        out.append(Alloc(ns, lo, hi, name, std, clause, page))
    return out


def module_path_to_file(mod_path: str) -> Path:
    parts = mod_path.split("::")
    if parts[0] == "crate":
        parts = parts[1:]
    base = SRC.joinpath(*parts)
    for cand in (base.with_suffix(".rs"), base / "mod.rs"):
        if cand.exists():
            return cand
    raise GenError(f"module `{mod_path}` has no source file")


def load_tables() -> list[Impl]:
    path = SRC / "tables" / "any.rs"
    body = strip_line_comments(macro_body(read(path), "declare_tables", path))
    head, _, tail = body.partition("@no_dispatch")
    out = []
    for m in re.finditer(r"(\w+)\s*=\s*\[([^\]]*)\]\s*=>\s*(crate::tables::[\w:]+)::(\w+)", head):
        variant, ranges, mod, ty = m.groups()
        rs = []
        for r in re.finditer(r"(0x[0-9A-Fa-f]+)\s*\.\.=\s*(0x[0-9A-Fa-f]+)", ranges):
            rs.append((int(r.group(1), 16), int(r.group(2), 16)))
        f = module_path_to_file(mod)
        out.append(Impl("table_id", rs, variant, ty, f, True, cite=citation(module_doc(f))))
    for m in re.finditer(r"(\w+)\s*=>\s*(crate::tables::[\w:]+)::(\w+)", tail):
        variant, mod, ty = m.groups()
        f = module_path_to_file(mod)
        tid = re.search(r"pub const TABLE_ID:\s*u8\s*=\s*(0x[0-9A-Fa-f]+)", read(f))
        if not tid:
            raise GenError(f"{rel(f)}: no `pub const TABLE_ID` for no-dispatch table {ty}")
        v = int(tid.group(1), 16)
        out.append(
            Impl(
                "table_id",
                [(v, v)],
                variant,
                ty,
                f,
                False,
                status="Full",
                note="typed view of the DSM-CC 0x3E range; reach it with `AnyTableSection::parse_as`",
                cite=citation(module_doc(f)),
            )
        )
    if not out:
        raise GenError("declare_tables! yielded no tables")
    for i in out:
        key = rel(i.module).removeprefix("src/")
        if key in TABLE_PARTIAL:
            i.status = "Partial"
            i.note = TABLE_PARTIAL[key]
    return out


# Descriptors whose typed struct deliberately keeps a sub-structure opaque.
# Keyed by module file (relative to dvb-si/src); the generator fails if a key
# no longer exists, so a rename cannot silently drop a note.
PARTIAL = {
    "descriptors/linkage.rs": "linkage_type 0x09/0x0A (SSU), 0x0B/0x0C (IP/MAC), 0x20 (TTML fonts) and user-defined conditional blocks kept as raw `LinkageData::Other`",
    "descriptors/data_broadcast.rs": "selector_byte kept raw (its meaning depends on data_broadcast_id)",
    "descriptors/data_broadcast_id.rs": "id_selector typed for SSU (`data_broadcast_id` 0x000A); other ids kept as `IdSelector::Raw`",
    "descriptors/extension/ac4.rs": "`ac4_toc()` kept opaque (its syntax is in ETSI TS 103 190-2; issue #102)",
    "descriptors/iod.rs": "`InitialObjectDescriptor()` kept opaque (ISO/IEC 14496-1)",
    "descriptors/muxcode.rs": "`MuxCodeTableEntry()` records kept opaque (ISO/IEC 14496-1)",
    "descriptors/fmx_buffer_size.rs": "`FlexMuxBufferDescriptor()` records kept as raw bytes (ISO/IEC 14496-1 §11.2)",
    "descriptors/avs2_audio.rs": "`anc_data_block()` kept in `trailing` (defined in a non-vendored standard)",
    "descriptors/mpeg4_text.rs": "`textConfig()` kept opaque (ISO/IEC 14496-17, not vendored)",
    "descriptors/mpeg4_audio_extension.rs": "`audioSpecificConfig()` kept opaque (ISO/IEC 14496-3, not vendored)",
}

# Sections whose payload is deliberately kept as bytes (status Partial, note shown).
TABLE_PARTIAL = {
    "tables/iso14496_section.rs": "payload (`SL_Packet()` / `M4MuxPacket()`s, selected by the carrying PMT) kept opaque (ISO/IEC 14496-1)",
    "tables/metadata_section.rs": "`metadata_byte`s (the metadata Access Unit) kept opaque",
    "tables/green_access_unit_section.rs": "`Green_AU()` kept opaque (its loops are bounded by the out-of-band Green extension descriptor)",
}

# Opaque MPEG-2 extension bodies whose payload is defined elsewhere (default: ISO/IEC 23008-3).
OPAQUE_EXT_NOTES = {
    "OdUpdate": "`ObjectDescriptorUpdate()` kept as bytes (ISO/IEC 14496-1 §7.2.5.5.2, not vendored)",
    "TimedMetadataExtension": "body kept as bytes (ISO/IEC 23001-13 clause 8.2.3, not vendored)",
}


def load_descriptors() -> tuple[list[Impl], list[Impl]]:
    path = SRC / "descriptors" / "any.rs"
    body = strip_line_comments(macro_body(read(path), "declare_descriptors", path))
    head, _, tail = body.partition("@no_dispatch")
    public, private = [], []
    for m in re.finditer(r"(\w+)\s*=\s*(0x[0-9A-Fa-f]+)\s*=>\s*(crate::descriptors::[\w:]+)::(\w+)", head):
        variant, tag, mod, ty = m.groups()
        f = module_path_to_file(mod)
        v = int(tag, 16)
        public.append(Impl("descriptor_tag", [(v, v)], variant, ty, f, True, cite=citation(module_doc(f))))
    for m in re.finditer(r"(\w+)\s*=>\s*(crate::descriptors::[\w:]+)::(\w+)", tail):
        variant, mod, ty = m.groups()
        f = module_path_to_file(mod)
        text = read(f)
        consts = dict(
            (k, int(v, 0)) for k, v in re.findall(r"pub const (TAG\w*):\s*u8\s*=\s*(0x[0-9A-Fa-f]+|\d+);", text)
        )
        if "TAG" in consts:
            tag = consts["TAG"]
        else:
            suffix = re.search(r"(V\d+)$", ty)
            key = f"TAG_{suffix.group(1)}" if suffix else None
            if key not in consts:
                raise GenError(f"{rel(f)}: cannot find the TAG const for private descriptor {ty}")
            tag = consts[key]
        private.append(Impl("private", [(tag, tag)], variant, ty, f, False, status="Opt-in", cite=citation(module_doc(f))))
    if not public or not private:
        raise GenError("declare_descriptors! yielded an empty dispatched or private list")
    return public, private


def load_registry_optins(private: list[Impl]) -> None:
    """Attach the DescriptorRegistry opt-in method + scope to each private descriptor."""
    path = SRC / "descriptors" / "registry.rs"
    text = read(path)
    methods = []
    for m in re.finditer(r"pub fn (with_\w+)\(&mut self[^)]*\)[^{]*\{(.*?)\n    \}", text, re.S):
        methods.append((m.group(1), m.group(2)))
    for d in private:
        hits = [name for name, body in methods if re.search(r"\b" + re.escape(d.rust_type) + r"\b", body)]
        if d.rust_type == "LogicalChannelDescriptor":
            hits = [n for n, b in methods if "logical_channel" in b and "nordig" not in b.lower()]
        if not hits:
            raise GenError(f"no DescriptorRegistry::with_* opt-in references {d.rust_type}")
        d.extra["optin"] = hits
        body = "\n".join(b for n, b in methods if n in hits)
        pds = sorted(set(re.findall(r"\bPDS_(\w+)\b", body)))
        if d.rust_type == "LogicalChannelDescriptor":
            scope = "any PDS (global) or a chosen PDS (`with_logical_channel_for_pds`, e.g. `PDS_EACEM` / `PDS_NORDIG`)"
        elif pds:
            scope = ", ".join(f"`PDS_{p}`" for p in pds) + " (private_data_specifier or registration_descriptor)"
        else:
            scope = "none — explicit, PDS-agnostic opt-in"
        d.extra["scope"] = scope


def load_extension_bodies() -> list[Impl]:
    path = SRC / "descriptors" / "extension" / "mod.rs"
    body = strip_line_comments(macro_body(read(path), "declare_extension_bodies", path))
    out = []
    for m in re.finditer(r"(\w+)\s*=\s*(0x[0-9A-Fa-f]+)\s*=>\s*(\w+)", body):
        variant, tag, ty = m.groups()
        f = find_type_file(ty, SRC / "descriptors" / "extension")
        v = int(tag, 16)
        out.append(Impl("dvb_extension_tag", [(v, v)], variant, ty, f, True, cite=citation(module_doc(f))))
    if not out:
        raise GenError("declare_extension_bodies! yielded nothing")
    return out


def load_ait() -> list[Impl]:
    path = SRC / "descriptors" / "ait" / "mod.rs"
    body = strip_line_comments(macro_body(read(path), "declare_ait_descriptors", path))
    out = []
    for m in re.finditer(r"(\w+)\s*=\s*(0x[0-9A-Fa-f]+)\s*=>\s*(crate::descriptors::[\w:]+)::(\w+)", body):
        variant, tag, mod, ty = m.groups()
        f = module_path_to_file(mod)
        v = int(tag, 16)
        out.append(Impl("ait_descriptor_tag", [(v, v)], variant, ty, f, True, cite=citation(module_doc(f))))
    if not out:
        raise GenError("declare_ait_descriptors! yielded nothing")
    return out


def load_scoped() -> list[Impl]:
    """The INT / UNT / RNT scoped namespaces (``declare_scoped_namespace!``)."""
    out = []
    for subdir, ns in (
        ("int", "int_descriptor_tag"),
        ("unt", "unt_descriptor_tag"),
        ("rnt", "rnt_descriptor_tag"),
    ):
        path = SRC / "descriptors" / subdir / "mod.rs"
        body = strip_line_comments(macro_body(read(path), "declare_scoped_namespace", path))
        n = 0
        for m in re.finditer(r"(\w+)\s*=\s*(0x[0-9A-Fa-f]+)\s*=>\s*(crate::descriptors::[\w:]+)::(\w+)", body):
            variant, tag, mod, ty = m.groups()
            f = module_path_to_file(mod)
            v = int(tag, 16)
            out.append(Impl(ns, [(v, v)], variant, ty, f, True, cite=citation(module_doc(f))))
            n += 1
        if not n:
            raise GenError(f"declare_scoped_namespace! in {rel(path)} yielded nothing")
    return out


def opaque_struct(ty: str, text: str) -> bool:
    m = re.search(r"pub struct " + re.escape(ty) + r"\b[^{;]*\{(.*?)\n\}", text, re.S)
    if not m:
        raise GenError(f"struct {ty} not found")
    fields = re.findall(r"pub \w+:\s*([^,\n]+),", m.group(1))
    return bool(fields) and all(t.strip() in ("Vec<u8>", "&'a [u8]") for t in fields)


def load_mpeg2_extension() -> list[Impl]:
    path = SRC / "descriptors" / "mpeg2_extension.rs"
    text = read(path)
    fn = re.search(r"pub const fn extension_descriptor_tag\(&self\) -> u8 \{(.*?)\n    \}", text, re.S)
    if not fn:
        raise GenError(f"{rel(path)}: Mpeg2ExtensionBody::extension_descriptor_tag not found")
    out = []
    for m in re.finditer(r"Self::(\w+)\(_\)\s*=>\s*(0x[0-9A-Fa-f]+)", fn.group(1)):
        variant, tag = m.groups()
        f = find_type_file(variant, SRC / "descriptors")
        v = int(tag, 16)
        opaque = opaque_struct(variant, read(f)) and variant != "AfExtensions"
        out.append(
            Impl(
                "mpeg2_extension_tag",
                [(v, v)],
                variant,
                variant,
                f,
                True,
                status="Typed-opaque" if opaque else "Full",
                note=OPAQUE_EXT_NOTES.get(variant, "payload kept as bytes (its syntax is defined in ISO/IEC 23008-3)") if opaque else "",
                cite=citation(module_doc(f)),
            )
        )
    if not out:
        raise GenError("Mpeg2ExtensionBody yielded no typed tags")
    return out


# Implementations living outside the declare_* lists (each verified to exist).
EXTRA = [
    ("dii_descriptor_tag", 0x09, "CompressedModuleDescriptor", "carousel/biop/message.rs",
     "decompressed with the `flate2` feature; parsed from DII `moduleInfo` userInfo by `ModuleInfo`",
     "ETSI TR 101 202 Table 4.14 (BIOP ModuleInfo userInfo)"),
]


def load_extra() -> list[Impl]:
    out = []
    for ns, tag, ty, file, note, cite in EXTRA:
        f = SRC / file
        if not re.search(r"^pub struct " + re.escape(ty) + r"\b", read(f), re.M):
            raise GenError(f"{file}: `pub struct {ty}` not found (update EXTRA in the generator)")
        out.append(Impl(ns, [(tag, tag)], ty, ty, f, True, note=note, cite=cite))
    return out


def apply_partial(impls: list[Impl]) -> None:
    by_file = {rel(i.module).removeprefix("src/"): i for i in impls}
    for key, note in PARTIAL.items():
        if not (SRC / key).exists():
            raise GenError(f"PARTIAL note for missing module src/{key} — update the generator")
        i = by_file.get(key)
        if i is None:
            raise GenError(f"PARTIAL note for src/{key}, which is not a descriptor module")
        i.status = "Opt-in, partial" if i.namespace == "private" else "Partial"
        i.note = note


# Annex A charset decoders: Charset variant -> (selector(s), table, decoding).
CHARSETS = {
    "Iso6937": ("none (first byte ≥ 0x20) or 0x00", "Figure A.1 default Latin (ISO/IEC 6937 superset, € at 0xA4)", "built-in, glyph-for-glyph; combining-diacritic row → precomposed or base + combining mark", "no"),
    "Iso8859": ("0x01–0x07, 0x09–0x0B; 0x10 0x00 nn", "ISO/IEC 8859-5 … -15 (no -12); two-byte form any -n", "`encoding_rs` (8859-1 built-in)", "yes, except 8859-1"),
    "Ucs2Be": ("0x11", "ISO/IEC 10646 Basic Multilingual Plane", "built-in UCS-2 big-endian", "no"),
    "Ksx1001": ("0x12", "KS X 1001 (Korean)", "`encoding_rs` EUC-KR", "yes"),
    "Gb2312": ("0x13", "GB-2312-1980 (Simplified Chinese)", "`encoding_rs` GBK (a GB-2312 superset)", "yes"),
    "Big5": ("0x14", "Big5 (Traditional Chinese)", "`encoding_rs` Big5", "yes"),
    "Utf8": ("0x15", "UTF-8 encoding of ISO/IEC 10646", "passthrough (lossy on invalid bytes)", "no"),
    "Unsupported": ("0x1F + encoding_type_id; reserved 0x08, 0x0C–0x0F, 0x16–0x1E", "no registered broadcast encodings / reserved", "U+FFFD (never panics)", "no"),
}


def load_charsets() -> list[str]:
    path = SRC / "text" / "mod.rs"
    m = re.search(r"enum Charset \{(.*?)\n\}", read(path), re.S)
    if not m:
        raise GenError(f"{rel(path)}: `enum Charset` not found")
    variants = re.findall(r"^\s{4}(\w+)", strip_line_comments(m.group(1)), re.M)
    if set(variants) != set(CHARSETS):
        raise GenError(
            f"{rel(path)}: Charset variants {sorted(variants)} differ from the generator's "
            f"CHARSETS {sorted(CHARSETS)} — update CHARSETS in tools/gen-dvbsi-coverage.py"
        )
    return variants


# DSM-CC / carousel layer: (type, what, spec).
CAROUSEL = [
    ("DsmccSection", "DSM-CC section framing, table_id 0x3A–0x3F (CRC or checksum)", "ISO/IEC 13818-6 §9 / ETSI EN 301 192"),
    ("Dsi", "DownloadServerInitiate (messageId 0x1006); privateData = GroupInfoIndication or ServiceGatewayInfo", "ISO/IEC 13818-6 §7.3 / TR 101 202"),
    ("GroupInfoIndication", "SSU / data-carousel group list carried in the DSI", "ETSI TS 102 006 / EN 301 192"),
    ("Dii", "DownloadInfoIndication (messageId 0x1002) with a typed module loop", "ISO/IEC 13818-6 §7.3"),
    ("DownloadDataBlock", "DownloadDataBlock (messageId 0x1003)", "ISO/IEC 13818-6 §7.3"),
    ("ModuleReassembler", "DDB → complete modules per DII geometry: version-aware, out-of-order tolerant, per-module + aggregate memory caps", "—"),
    ("CompatibilityDescriptor", "compatibilityDescriptor() of DSI/DII groups and UNT platforms", "ETSI TS 102 006 Table 15 / ISO/IEC 13818-6"),
    ("ServiceGatewayInfo", "object-carousel DSI privateData", "TR 101 202 Table 4.15"),
    ("ModuleInfo", "BIOP ModuleInfo (DII moduleInfoBytes), incl. compressed_module", "TR 101 202 Table 4.14"),
    ("BiopMessage", "BIOP Directory / File / ServiceGateway / Stream / StreamEvent messages", "ISO/IEC 13818-6 §11 / TR 101 202 §4.7"),
    ("Ior", "IOP::IOR with tagged BIOP / Lite-Options profile bodies, object locations, taps", "ISO/IEC 13818-6 §11 / TR 101 202"),
    ("CarouselFs", "virtual file tree over a reassembled object carousel (`resolve` / `file_bytes`)", "—"),
]


def load_carousel() -> list[tuple[str, str, str, Path]]:
    out = []
    for ty, what, spec in CAROUSEL:
        out.append((ty, what, spec, find_type_file(ty, SRC)))
    return out


# Higher layers (verified to exist): (path, item, what).
LAYERS = [
    ("demux.rs", "SiDemux", "PID-filtered, CRC-checked, version-gated section pump over 188-byte TS packets; follows the PAT to PMT PIDs", "ts"),
    ("collect/mod.rs", "SectionSetCollector", "assemble every long-form table across `section_number 0..=last_section_number`", ""),
    ("collect/eit.rs", "EitCollector", "EIT schedule collection across `last_table_id` (the rule section numbers alone miss)", ""),
    ("collect/nit.rs", "CompleteNit", "flattened logical NIT (also `CompleteBat`, `CompleteSdt`, `CompleteEit`)", ""),
    ("epg.rs", "EpgStore", "bounded EPG: deduplicated events, decoded extended text, now/next, SDT name join", "chrono"),
    ("descriptors/registry.rs", "DescriptorRegistry", "runtime registration of private descriptor tags, PDS-scoped", ""),
    ("descriptors/extension/registry.rs", "ExtensionRegistry", "runtime registration of private descriptor_tag_extension bodies", ""),
    ("tables/registry.rs", "TableRegistry", "runtime registration of private table_ids", ""),
    ("owned.rs", "Owned", "keep a parsed zero-copy view past its buffer's borrow (`Send + Sync`, `'static`)", "yoke"),
]


def load_layers() -> list[tuple[str, str, str, str]]:
    for file, item, _, _ in LAYERS:
        if not re.search(r"^pub (?:struct|enum) " + item + r"\b", read(SRC / file), re.M):
            raise GenError(f"src/{file}: `pub struct {item}` not found (update LAYERS in the generator)")
    return LAYERS


def registry_counts() -> dict[str, int]:
    def count(path: Path, section: str) -> int:
        n, on = 0, False
        for line in read(path).splitlines():
            if line.startswith("["):
                on = line.strip() == f"[{section}]"
                continue
            if on and re.match(r"^0x[0-9A-Fa-f]", line):
                n += 1
        return n

    return {
        "cas": count(CRATE / "registries" / "tsCAS.names", "CASystemId"),
        "pds": count(CRATE / "registries" / "tsPDS.names", "PrivateDataSpecifier"),
    }


# Spec-transcription directories under dvb-si/docs/{kind}/<dir>/ -> document title.
# A new directory without an entry fails generation, so the index cannot rot.
SPEC_DIRS = {
    "en_300_468": "ETSI EN 300 468 — DVB Service Information",
    "en_300_706": "ETSI EN 300 706 — Enhanced Teletext",
    "en_300_743": "ETSI EN 300 743 — DVB subtitling",
    "en_301_192": "ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel)",
    "en_303_560": "ETSI EN 303 560 — DVB TTML subtitling",
    "gy_t_270": "GY/T 270-2013 — China DTV closed captions",
    "gy_t_420": "GY/T 420-2025 — China UHD encapsulation (AVS2 video)",
    "gy_t_422": "GY/T 422-2025 — China multi-audio (audio preselection profile)",
    "iso_13818_1": "Rec. ITU-T H.222.0 \\| ISO/IEC 13818-1 — MPEG-2 Systems",
    "iso_13818_6": "ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented)",
    "iso_14496_1": "ISO/IEC 14496-1 — MPEG-4 Systems (FlexMux buffer descriptors)",
    "nordig": "NorDig Unified Requirements v3.2.2 — logical channel descriptors",
    "t_ai_109_6": "T/AI 109.6-2022 — AVS3 intelligent media format",
    "t_ai_109_7": "T/AI 109.7-2024 — AVS3 audio encapsulation",
    "t_uwa_012_2": "T/UWA 012.2-2023 — UWA UHD TS encapsulation",
    "tr_101_211": "ETSI TR 101 211 — DVB SI implementation guidelines",
    "tr_101_290": "ETSI TR 101 290 — DVB measurement guidelines",
    "ts_101_154": "ETSI TS 101 154 — audio/video coding in DVB",
    "ts_101_162": "ETSI TS 101 162 — DVB identifier registries",
    "ts_102_006": "ETSI TS 102 006 — System Software Update",
    "ts_102_323": "ETSI TS 102 323 — TV-Anytime carriage",
    "ts_102_366": "ETSI TS 102 366 — AC-3 / E-AC-3",
    "ts_102_727": "ETSI TS 102 727 — DVB-MHP (descriptor subset)",
    "ts_102_772": "ETSI TS 102 772 — MPE-IFEC",
    "ts_102_809": "ETSI TS 102 809 — application signalling (AIT)",
    "ts_102_825": "ETSI TS 102 825 — CPCM",
}
SPEC_KINDS = [
    ("tables", "Table / section syntax"),
    ("descriptors", "Descriptor syntax"),
    ("enums", "Coded-value tables (with `.toml` drift guards)"),
    ("text", "Prose references"),
]
DOCS_INDEX = CRATE / "docs" / "README.md"
INDEX_BEGIN = "<!-- BEGIN GENERATED: dvb-si-docs-index (tools/gen-dvbsi-coverage.py) -->"
INDEX_END = "<!-- END GENERATED: dvb-si-docs-index -->"


def render_docs_index() -> list[str]:
    out = [INDEX_BEGIN, "", "| Kind | Standard | Files |", "|---|---|---|"]
    for kind, label in SPEC_KINDS:
        base = CRATE / "docs" / kind
        for d in sorted(p for p in base.iterdir() if p.is_dir()):
            if d.name not in SPEC_DIRS:
                raise GenError(f"docs/{kind}/{d.name}/ has no title in SPEC_DIRS (tools/gen-dvbsi-coverage.py)")
            n = sum(1 for f in d.rglob("*.md") if f.name != "README.md")
            out.append(f"| {label} | [{SPEC_DIRS[d.name]}]({kind}/{d.name}/) | {n} |")
    out += ["", INDEX_END]
    return out


# ─────────────────────────── standards ─────────────────────────────

# Standards dvb-si implements, in display order. `match` selects the
# allocations.tsv rows (by `standard` prefix) whose implemented/total ratio
# gives the status; standards with no allocation list carry a fixed status.
STANDARDS = [
    {
        "id": "ETSI EN 300 468",
        "title": "DVB Service Information (SI)",
        "edition": "V1.20.1 (2026-09); RFU defaults audited back to V1.3.1",
        "scope": "all SI tables (NIT, BAT, SDT, EIT, TDT, TOT, RST, ST, SAT, DIT, SIT), Table 12 descriptors 0x40–0x7F, Table 109 extension descriptors, Annex A text, Annexes D/G/H/L audio descriptors",
        "match": ["ETSI EN 300 468"],
    },
    {
        "id": "Rec. ITU-T H.222.0 \\| ISO/IEC 13818-1",
        "title": "MPEG-2 Systems — PSI and descriptors",
        "edition": "08/2023 text (clause numbers in COVERAGE.md); MPEG-H 3D audio descriptors 06/2021",
        "scope": "PAT, CAT, PMT, TSDT; Table 2-45 descriptors; Table 2-110 extension descriptors",
        "match": ["ITU-T H.222.0"],
    },
    {
        "id": "ISO/IEC 13818-6",
        "title": "DSM-CC (data/object carousel)",
        "edition": "as profiled by ETSI TR 101 202 V1.2.1 (13818-6 itself is not vendored)",
        "scope": "DSM-CC sections, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel_identifier + association_tag descriptors",
        "match": ["ISO/IEC 13818-6"],
    },
    {
        "id": "ETSI EN 301 192",
        "title": "DVB data broadcasting",
        "edition": "V1.7.1",
        "scope": "MPE datagram, MPE-FEC, INT section; time_slice_fec_identifier + ECM_repetition_rate descriptors; DII carousel descriptors",
        "match": ["ETSI EN 301 192"],
    },
    {
        "id": "ETSI TS 102 006",
        "title": "DVB System Software Update (SSU)",
        "edition": "V1.7.1 (some module docs still cite V1.4.1)",
        "scope": "UNT section, compatibilityDescriptor, SSU data_broadcast_id selector",
        "match": ["ETSI TS 102 006"],
    },
    {
        "id": "ETSI TS 102 809",
        "title": "Application signalling (HbbTV AIT)",
        "edition": "V1.3.1",
        "scope": "AIT section, AIT descriptor namespace, application_signalling + service_identifier descriptors, protection message section + descriptor",
        "match": ["ETSI TS 102 809"],
    },
    {
        "id": "ETSI TS 102 727",
        "title": "DVB-MHP 1.1.3",
        "edition": "V1.1.1 (descriptor subset)",
        "scope": "DVB-J application + location AIT descriptors, XAIT_location, XAIT_PID",
        "match": ["ETSI TS 102 727"],
    },
    {
        "id": "ETSI TS 102 323",
        "title": "TV-Anytime carriage in DVB",
        "edition": "V1.4.1",
        "scope": "container, RCT, CIT, RNT sections; default_authority, related_content, TVA_id, content_identifier descriptors",
        "match": ["ETSI TS 102 323"],
    },
    {
        "id": "ETSI TS 102 772",
        "title": "MPE-IFEC",
        "edition": "V1.1.1",
        "scope": "MPE-IFEC section",
        "match": ["ETSI TS 102 772"],
    },
    {
        "id": "ETSI EN 303 560",
        "title": "DVB TTML subtitling",
        "edition": "V1.1.1",
        "scope": "TTML_subtitling descriptor, downloadable font info section (DFIS)",
        "match": ["ETSI EN 303 560"],
    },
    {
        "id": "ETSI TS 102 825-9",
        "title": "CPCM system adaptation",
        "edition": "V1.2.1",
        "scope": "cpcm_delivery_signalling extension descriptor",
        "match": ["ETSI TS 102 825-9"],
    },
    {
        "id": "ETSI TS 101 154",
        "title": "Audio/video coding in DVB",
        "edition": "V2.10.1",
        "scope": "PMT `StreamType` values incl. AVS2/AVS3 (0xD2–0xD5); AVS private_data_specifier rules",
        "fixed": "Partial",
    },
    {
        "id": "ETSI TS 101 162",
        "title": "DVB identifiers",
        "edition": "registry data from TSDuck (see registries/README.md)",
        "scope": "CA_system_id and private_data_specifier value → name lookups",
        "fixed": "Partial",
    },
    {
        "id": "NorDig Unified Requirements",
        "title": "Nordic IRD requirements",
        "edition": "v3.2.2 (2025-11)",
        "scope": "logical channel descriptor v1 (0x83) + v2 (0x87), PDS 0x00000029",
        "fixed": "Full (scoped)",
    },
    {
        "id": "EACEM / D-Book",
        "title": "Logical channel numbering",
        "edition": "private (PDS 0x00000028); no vendored spec",
        "scope": "logical_channel descriptor (0x83)",
        "fixed": "Full (scoped)",
    },
    {
        "id": "GY/T 270-2013",
        "title": "China DTV closed captions",
        "edition": "2013",
        "scope": "caption_service_descriptor (0x86), `StreamType::ChinaClosedCaption`",
        "fixed": "Full (scoped)",
    },
    {
        "id": "GY/T 420-2025, GY/T 422-2025",
        "title": "China UHD encapsulation / multi-audio",
        "edition": "2025",
        "scope": "AVS2_video (0x40 under 'AVSV'), AVS3_video; GY/T 422 audio_preselection profile",
        "fixed": "Full (scoped)",
    },
    {
        "id": "T/AI 109.6-2022, T/AI 109.7-2024",
        "title": "AVS3 media format / audio transport",
        "edition": "2022 / 2024",
        "scope": "AVS3_video (0xD1), AVS3_audio (0xD2), AVS2_audio (0xD3)",
        "fixed": "Full (scoped)",
    },
    {
        "id": "T/UWA 012.2-2023",
        "title": "UWA UHD TS encapsulation",
        "edition": "2023",
        "scope": "older AVS3 video descriptor (tag 62)",
        "fixed": "Full (scoped)",
    },
]


# ─────────────────────────── assembly ──────────────────────────────


@dataclass
class Model:
    tables: list[Impl]
    descriptors: list[Impl]
    private: list[Impl]
    ext: list[Impl]
    mpeg2ext: list[Impl]
    ait: list[Impl]
    scoped: list[Impl]
    extra: list[Impl]
    allocs: list[Alloc]
    charsets: list[str]
    carousel: list
    layers: list
    registries: dict[str, int]

    def impls(self, ns: str) -> list[Impl]:
        pool = self.tables + self.descriptors + self.ext + self.mpeg2ext + self.ait + self.scoped + self.extra
        return [i for i in pool if i.namespace == ns]

    def allocs_in(self, ns: str) -> list[Alloc]:
        return [a for a in self.allocs if a.namespace == ns]

    def impl_for(self, a: Alloc) -> Impl | None:
        for i in self.impls(a.namespace):
            if i.covers(a.lo, a.hi):
                return i
        return None


def build() -> Model:
    tables = load_tables()
    descriptors, private = load_descriptors()
    load_registry_optins(private)
    ext = load_extension_bodies()
    mpeg2ext = load_mpeg2_extension()
    ait = load_ait()
    scoped = load_scoped()
    extra = load_extra()
    apply_partial(descriptors + ext + private)
    allocs = load_allocations()
    m = Model(
        tables, descriptors, private, ext, mpeg2ext, ait, scoped, extra, allocs,
        load_charsets(), load_carousel(), load_layers(), registry_counts(),
    )
    # Every implemented code must appear in the allocation list of its
    # namespace — otherwise allocations.tsv is incomplete (or the code is wrong).
    for ns in ("table_id", "descriptor_tag", "dvb_extension_tag", "mpeg2_extension_tag", "ait_descriptor_tag", "dii_descriptor_tag", "int_descriptor_tag", "unt_descriptor_tag", "rnt_descriptor_tag"):
        allocs_ns = m.allocs_in(ns)
        for i in m.impls(ns):
            for lo, hi in i.ranges:
                for v in range(lo, hi + 1):
                    if not any(a.lo <= v <= a.hi for a in allocs_ns):
                        raise GenError(
                            f"{i.rust_type} implements {ns} 0x{v:02X}, which allocations.tsv does not list"
                        )
    return m


NOT_IMPL_NOTE = {
    "table_id": "dispatched as `AnyTableSection::Unknown` (section bytes kept)",
    "descriptor_tag": "surfaces as `AnyDescriptor::Unknown { tag, body }` — bytes kept, round-trips losslessly",
    "dvb_extension_tag": "kept as `ExtensionBody::Raw` — round-trips losslessly",
    "mpeg2_extension_tag": "kept as `Mpeg2ExtensionBody::Other` — round-trips losslessly",
    "ait_descriptor_tag": "surfaces as `AnyAitDescriptor::Unknown` — bytes kept",
    "int_descriptor_tag": "surfaces as `AnyIntDescriptor::Unknown` — bytes kept",
    "unt_descriptor_tag": "surfaces as `AnyUntDescriptor::Unknown` — bytes kept",
    "dii_descriptor_tag": "kept as raw bytes inside `ModuleInfo` / DII `moduleInfo`",
    "rnt_descriptor_tag": "decoded through the SI dispatch (`AnyRntDescriptor::Si`) — bytes kept",
}


def status_counts(m: Model, ns: str) -> tuple[int, int]:
    rows = m.allocs_in(ns)
    done = sum(1 for a in rows if m.impl_for(a))
    return done, len(rows)


def ids_covered(m: Model) -> int:
    ids = set()
    for i in m.tables:
        for lo, hi in i.ranges:
            ids.update(range(lo, hi + 1))
    return len(ids)


def standard_status(m: Model, s: dict) -> tuple[str, int, int]:
    if "fixed" in s:
        return s["fixed"], 0, 0
    rows = [a for a in m.allocs if any(a.standard.startswith(p) for p in s["match"])]
    done = sum(1 for a in rows if m.impl_for(a))
    if not rows:
        raise GenError(f"standard {s['id']} matches no allocations.tsv rows")
    if done == len(rows):
        return "Full", done, len(rows)
    if done == 0:
        return "Not implemented", done, len(rows)
    return "Partial", done, len(rows)


def counts(m: Model) -> dict:
    nd = [i for i in m.descriptors]
    mpeg = [i for i in nd if i.ranges[0][0] < 0x40]
    dvb = [i for i in nd if i.ranges[0][0] >= 0x40]
    not_impl = sum(1 for a in m.allocs if not m.impl_for(a))
    std_ids = len(STANDARDS)
    return {
        "sections": len(m.tables),
        "table_ids": ids_covered(m),
        "desc_total": len(nd),
        "desc_mpeg": len(mpeg),
        "desc_dvb": len(dvb),
        "ext": len(m.ext),
        "mpeg2ext": len(m.mpeg2ext),
        "ait": len(m.ait),
        "scoped": len({i.rust_type for i in m.scoped}),
        "private": len(m.private),
        "charsets": len([c for c in m.charsets if c != "Unsupported"]),
        "carousel": len(m.carousel),
        "standards": std_ids,
        "not_impl": not_impl,
        "cas": m.registries["cas"],
        "pds": m.registries["pds"],
        "typed_total": len(m.tables) + len(nd) + len(m.ext) + len(m.mpeg2ext) + len(m.ait) + len({i.rust_type for i in m.scoped}) + len(m.private),
    }


# ─────────────────────────── rendering ─────────────────────────────


def at_a_glance_rows(c: dict) -> list[tuple[str, str]]:
    return [
        ("Section types (table_id)", f"**{c['sections']}** typed `*Section` parsers covering **{c['table_ids']}** table_id values — every allocated EN 300 468 Table 2 id"),
        ("Descriptors (tag 0x00–0x7F)", f"**{c['desc_total']}** auto-dispatched: **{c['desc_mpeg']}** MPEG-2 / DSM-CC (0x02–0x3F) + all **{c['desc_dvb']}** DVB tags (0x40–0x7F)"),
        ("DVB extension descriptors (0x7F)", f"**{c['ext']}** typed `descriptor_tag_extension` bodies — every one EN 300 468 V1.20.1 Table 109 allocates"),
        ("MPEG-2 extension descriptors (0x3F)", f"**{c['mpeg2ext']}** typed `extension_descriptor_tag` bodies (MPEG-H 3D audio)"),
        ("AIT descriptors (TS 102 809 / MHP)", f"**{c['ait']}** typed in the separate AIT descriptor namespace"),
        ("INT / UNT / RNT descriptors", f"**{c['scoped']}** typed in their own scoped namespaces (`AnyIntDescriptor`, `AnyUntDescriptor`, `AnyRntDescriptor`)"),
        ("Private / regional descriptors", f"**{c['private']}** PDS- or registration-scoped opt-ins (NorDig, EACEM/D-Book, China GY/T + T/AI AVS2/AVS3, T/UWA)"),
        ("Text (EN 300 468 Annex A)", f"**{c['charsets']}** charset decoders covering every Table A.3 selector + Annex A.1 control codes"),
        ("DSM-CC carousel", f"**{c['carousel']}** layers: section framing, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel file tree"),
        ("Registries", f"**{c['cas']}** CA_system_id and **{c['pds']}** private_data_specifier names (TSDuck data, build-time)"),
        ("Standards", f"**{c['standards']}** standards / families — see the table below"),
        ("Known gaps", f"**{c['not_impl']}** allocated codes not yet typed — listed in [`docs/COVERAGE.md`]({REPO_BLOB}docs/COVERAGE.md#not-implemented)"),
    ]


def standards_table(m: Model, link_prefix: str) -> list[str]:
    out = [
        "| Standard | Title | Edition implemented | Scope covered | Status |",
        "|---|---|---|---|---|",
    ]
    for s in STANDARDS:
        st, done, total = standard_status(m, s)
        tally = f" ({done}/{total} allocated codes)" if total else ""
        out.append(f"| {s['id']} | {s['title']} | {s['edition']} | {s['scope']} | {st}{tally} |")
    return out


def glance_block(m: Model, link_prefix: str) -> list[str]:
    c = counts(m)
    out = [
        "| | |",
        "|---|---|",
    ]
    out += [f"| {k} | {v} |" for k, v in at_a_glance_rows(c)]
    out += ["", "### Standards supported", ""]
    out += standards_table(m, link_prefix)
    out += [
        "",
        "Status: **Full** = every code the edition allocates is typed; **Partial** = some are"
        " (the tally counts allocated codes, so a single untyped one shows); **Full (scoped)** ="
        " typed, but only decoded inside the private_data_specifier / registration scope the standard"
        " requires (opt in via `DescriptorRegistry`); **Not implemented** = bytes are preserved"
        " but not typed. Every count above is generated by `tools/gen-dvbsi-coverage.py` from the"
        " crate's dispatch lists and CI fails if it goes stale.",
    ]
    return out


def table_rows(m: Model) -> list[str]:
    out = [
        "| table_id | Section | Standard | Clause (verified) | Source cites | Rust type | Status |",
        "|---|---|---|---|---|---|---|",
    ]
    for a in m.allocs_in("table_id"):
        i = m.impl_for(a)
        if i:
            ty = md_link(i.module, i.rust_type)
            st = i.status + (f" — {i.note}" if i.note else "")
            src = i.cite
        else:
            ty, st, src = "—", "Not implemented — " + NOT_IMPL_NOTE["table_id"], "—"
        out.append(f"| {fmt_code(a.lo, a.hi)} | {a.name} | {a.standard} | {a.clause or '—'} | {src} | {ty} | {st} |")
    return out


def tag_rows(m: Model, ns: str, lo: int = 0, hi: int = 0xFF, label: str = "tag") -> list[str]:
    out = [
        f"| {label} | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |",
        "|---|---|---|---|---|---|---|",
    ]
    for a in m.allocs_in(ns):
        if not (lo <= a.lo <= hi):
            continue
        i = m.impl_for(a)
        if i:
            ty = md_link(i.module, i.rust_type)
            st = i.status + (f" — {i.note}" if i.note else "")
            src = i.cite
        else:
            ty = "—"
            st = "Not implemented — " + NOT_IMPL_NOTE[ns].replace("{namespace}", str(ISSUES["namespace"]))
            src = "—"
        out.append(f"| {fmt_code(a.lo, a.hi)} | {a.name} | {a.standard} | {a.clause or '—'} | {src} | {ty} | {st} |")
    return out


def private_rows(m: Model) -> list[str]:
    out = [
        "| tag | Rust type | Standard (source cites) | Scope | Opt-in | Status |",
        "|---|---|---|---|---|---|",
    ]
    for d in sorted(m.private, key=lambda d: (d.ranges[0][0], d.rust_type)):
        opt = ", ".join(f"`DescriptorRegistry::{o}()`" for o in d.extra["optin"])
        st = d.status + (f" — {d.note}" if d.note else "")
        out.append(f"| {fmt_code(*d.ranges[0])} | {md_link(d.module, d.rust_type)} | {d.cite} | {d.extra['scope']} | {opt} | {st} |")
    return out


def not_impl_summary(m: Model) -> list[str]:
    groups = {}
    for a in m.allocs:
        if not m.impl_for(a):
            groups.setdefault(a.namespace, []).append(a)
    labels = {
        "table_id": "table_id",
        "descriptor_tag": "Descriptor tags (SI / MPEG namespace)",
        "dvb_extension_tag": "DVB descriptor_tag_extension",
        "mpeg2_extension_tag": "MPEG-2 extension_descriptor_tag",
        "ait_descriptor_tag": "AIT descriptors",
        "int_descriptor_tag": "INT descriptors",
        "unt_descriptor_tag": "UNT (SSU) descriptors",
        "dii_descriptor_tag": "DSM-CC data-carousel (DII/DSI) descriptors",
        "rnt_descriptor_tag": "RNT-scoped descriptors",
    }
    out = ["| Namespace | Count | Codes |", "|---|---|---|"]
    for ns, rows in groups.items():
        codes = ", ".join(f"{fmt_code(a.lo, a.hi)} {a.name}" for a in rows)
        out.append(f"| {labels[ns]} | {len(rows)} | {codes} |")
    return out


# Citations that may legitimately name a clause other than the verified one. Empty on
# purpose: a module doc must cite the clause that DEFINES its structure, in the edition
# the Standard column names (issue #1287). Add an entry only with a reason, keyed by
# (namespace, code) -> why.
CITATION_ALLOWLIST: dict[tuple[str, int], str] = {}


def citation_mismatches(m: Model) -> list[tuple[Alloc, Impl, set[str]]]:
    """Modules whose ``//!`` citation names no clause equal to (or a sub-clause of) the verified one.

    Strict: citing only an enclosing parent clause (``§6.2`` for ``6.2.27``) is also a
    disagreement, because the parent does not say which sub-clause defines the structure.
    """
    out = []
    for a in m.allocs:
        i = m.impl_for(a)
        if not i or not a.clause or (a.namespace, a.lo) in CITATION_ALLOWLIST:
            continue
        named = cited_clauses(i.cite)
        if not named:
            continue
        if not any(c == a.clause or c.startswith(a.clause + ".") for c in named):
            out.append((a, i, named))
    return out


# Implemented rows whose clause may stay unverified (owner-approved, with the reason).
# Empty: every clause an implemented module is checked against must have been read off the
# rendered PDF page (`verified_page` in allocations.tsv), not trusted from converted text.
UNVERIFIED_ALLOWLIST: dict[tuple[str, int], str] = {}


def unverified_rows(m: Model) -> list[tuple[Alloc, Impl]]:
    """Implemented rows carrying a clause but no ``verified_page`` (the strict check would trust them)."""
    return [
        (a, i)
        for a in m.allocs
        if a.clause and not a.verified_page and (a.namespace, a.lo) not in UNVERIFIED_ALLOWLIST
        for i in [m.impl_for(a)]
        if i
    ]


def cross_check(m: Model) -> list[str]:
    out = [
        f"| {a.namespace} {fmt_code(a.lo, a.hi)} | {md_link(i.module)} | {', '.join(sorted(named))} | {a.clause} ({a.standard}) |"
        for a, i, named in citation_mismatches(m)
    ]
    n_ver = sum(1 for a in m.allocs if a.clause and a.verified_page and m.impl_for(a))
    n_not = sum(1 for a in m.allocs if a.clause and not a.verified_page and not m.impl_for(a))
    if not out:
        return [
            "Every module citation that names a clause agrees with the verified clause.",
            "",
            f"{n_ver} implemented rows have their clause verified against the rendered PDF page",
            f"(`verified_page` column of `coverage/allocations.tsv`); {n_not} not-implemented rows",
            "carry a clause that is not yet page-verified (nothing cites it).",
        ]
    return [
        "| Code | Module | Module doc cites | Verified clause (edition) |",
        "|---|---|---|---|",
    ] + out


def render_coverage(m: Model) -> str:
    c = counts(m)
    L = []
    L += [
        "# dvb-si coverage reference",
        "",
        "<!-- GENERATED FILE — do not edit by hand. Regenerate with",
        "     `python3 tools/gen-dvbsi-coverage.py`; `cargo test -p dvb-si --test docs_coverage_drift`",
        "     fails when this file is stale. -->",
        "",
        "Everything `dvb-si` parses **and** serializes, generated from the crate's own dispatch lists",
        "(`declare_tables!`, `declare_descriptors!`, `declare_extension_bodies!`,",
        "`declare_ait_descriptors!`, `Mpeg2ExtensionBody`) and each module's `//!` spec citation,",
        "joined with the allocation lists the vendored standards define",
        "([`coverage/allocations.tsv`](coverage/allocations.tsv)). Rows the standards allocate but the",
        "crate does not type are listed as **Not implemented** — nothing is hidden.",
        "",
        "Columns: **Clause (verified)** is the clause of the named edition, checked against the spec",
        "text; **Source cites** is what the module's own doc comment says (see the",
        "[citation cross-check](#citation-cross-check) for disagreements). Status legend:",
        "",
        "- **Full** — typed struct, symmetric `Parse`/`Serialize`, round-trip tested, auto-dispatched.",
        "- **Partial** — typed, but a nested sub-structure is deliberately kept as bytes (reason given).",
        "- **Typed-opaque** — dispatched to its own type, whose payload is a byte run defined by another standard.",
        "- **Opt-in** — typed, but only decoded inside the private_data_specifier / registration scope",
        "  its standard requires; enable it on a `DescriptorRegistry`.",
        "- **Not implemented** — the bytes are preserved (round-trip stays lossless) but not typed.",
        "",
        "## Summary",
        "",
        "| | |",
        "|---|---|",
    ]
    L += [f"| {k} | {v} |" for k, v in at_a_glance_rows(c)]
    L += ["", "## Standards", ""]
    L += standards_table(m, "")
    L += ["", "## Tables (table_id)", ""]
    L += [
        "`AnyTableSection::parse` dispatches every implemented row except the MPE datagram view;",
        "`TableRegistry` adds private table_ids at runtime. User-defined (0x80–0xFE) and reserved",
        "values are omitted.",
        "",
    ]
    L += table_rows(m)
    L += ["", "## Descriptors — MPEG-2 / DSM-CC (0x00–0x3F)", ""]
    L += [
        "Allocated by Rec. ITU-T H.222.0 Table 2-45. Implemented tags are dispatched by",
        "`AnyDescriptor` / `DescriptorLoop::iter` / `parse_loop`.",
        "",
    ]
    L += tag_rows(m, "descriptor_tag", 0x00, 0x3F)
    L += ["", "## Descriptors — DVB (0x40–0x7F)", ""]
    L += ["Allocated by ETSI EN 300 468 V1.20.1 Table 12.", ""]
    L += tag_rows(m, "descriptor_tag", 0x40, 0x7F)
    L += ["", "## DVB extension descriptors (tag 0x7F, `descriptor_tag_extension`)", ""]
    L += [
        "Allocated by ETSI EN 300 468 V1.20.1 Table 109 (0x12 and 0x1A–0x1F are reserved there).",
        "`ExtensionRegistry` registers private extension bodies.",
        "",
    ]
    L += tag_rows(m, "dvb_extension_tag", label="tag_ext")
    L += ["", "## MPEG-2 extension descriptors (tag 0x3F, `extension_descriptor_tag`)", ""]
    L += ["Allocated by Rec. ITU-T H.222.0 (08/2023) Table 2-110.", ""]
    L += tag_rows(m, "mpeg2_extension_tag", label="ext_tag")
    L += ["", "## Private and regional descriptors (PDS / registration scoped)", ""]
    L += [
        "These tags are private (0x80–0xFE) or re-use a public tag inside a private scope, so they",
        "are **never auto-dispatched**: outside their scope they stay `AnyDescriptor::Unknown` (or",
        "the public descriptor — e.g. 0x40 stays `network_name`). Enable them on a",
        "`DescriptorRegistry`; a `private_data_specifier_descriptor` earlier in the same loop (or a",
        "`registration_descriptor` for the AVS family) sets the scope automatically.",
        "",
    ]
    L += private_rows(m)
    L += ["", "## AIT descriptors (own tag namespace)", ""]
    L += [
        "Walked by `parse_ait_loop` / `AnyAitDescriptor`. Allocated by ETSI TS 102 809 V1.3.1",
        "Table 38 (HbbTV) plus the MHP-only tags of ETSI TS 102 727 V1.1.1.",
        "",
    ]
    L += tag_rows(m, "ait_descriptor_tag")
    L += ["", "## INT, UNT, data-carousel and RNT scoped descriptors", ""]
    L += [
        "These standards define their own descriptor tag spaces (EN 301 192 and TS 102 006 forbid",
        "MPEG descriptors 0x00–0x3F in the INT/UNT; TS 102 323 Table 4 overrides 0x40–0x42 inside the",
        "RNT). Each table's loops are walked by its own namespace (`parse_int_loop` / `AnyIntDescriptor`,",
        "`parse_unt_loop` / `AnyUntDescriptor`, `parse_rnt_loop` / `AnyRntDescriptor`), which falls back",
        "to the SI dispatch (`Si`) for the tags the spec hands back to EN 300 468. Of the DSM-CC",
        "data-carousel descriptors only `compressed_module_descriptor` is typed today.",
        "",
        "### INT (ETSI EN 301 192 V1.7.1 Table 19)",
        "",
    ]
    L += tag_rows(m, "int_descriptor_tag")
    L += ["", "### UNT / SSU (ETSI TS 102 006 V1.7.1 Table 19)", ""]
    L += tag_rows(m, "unt_descriptor_tag")
    L += ["", "### DSM-CC data carousel (ETSI EN 301 192 V1.7.1 Table 48)", ""]
    L += tag_rows(m, "dii_descriptor_tag")
    L += ["", "### RNT scope (ETSI TS 102 323 V1.4.1 Table 4)", ""]
    L += tag_rows(m, "rnt_descriptor_tag")
    L += ["", "## DSM-CC data and object carousel", ""]
    L += ["| Type | What it is | Spec |", "|---|---|---|"]
    for ty, what, spec, f in m.carousel:
        L.append(f"| {md_link(f, ty)} | {what} | {spec} |")
    L += ["", "## Text (EN 300 468 Annex A)", ""]
    L += [
        "`DvbText` keeps the wire bytes and decodes on demand (`decode()`, `Display`, serde);",
        "`LangCode` is the ISO 639-2 / ISO 3166 three-byte code. Annex A.1 control codes are honoured",
        "for both the single-byte (0x80–0x9F) and two-byte (Table A.2) forms: emphasis dropped,",
        "CR/LF → space, reserved controls stripped.",
        "",
        "| Selector (Table A.3) | Character table | Decoder | Needs `std` |",
        "|---|---|---|---|",
    ]
    for v in m.charsets:
        sel, table, dec, std = CHARSETS[v]
        L.append(f"| {sel} | {table} | {dec} | {std} |")
    L += ["", "## Time and dates", ""]
    L += [
        "- **MJD + BCD UTC** (EN 300 468 Annex C) for EIT `start_time`, TDT/TOT `UTC_time`,",
        "  local_time_offset `time_of_change`, network_change_notify `start_time_of_change`:",
        "  always decoded to `broadcast_common::time::MjdBcdDateTime` (e.g. `EitEvent::start_time()`,",
        "  `TdtSection::utc_time_decoded()`), additionally to `chrono::DateTime<Utc>` with the",
        "  `chrono` feature (`start_time_chrono()`, `utc_time()`); `*_raw()` returns the wire bytes.",
        "- **2038 MJD wrap rule** (EN 300 468 V1.20.1 clauses 5.2.4–5.2.6, 6.2.20, 6.4.9): an MJD",
        "  whose most significant bit is 0 is read as `0x10000 + MJD`.",
        "- **BCD durations**: EIT `duration()` → `core::time::Duration`; local_time_offset",
        "  `local_time_offset()` / `next_time_offset()` → `chrono::Duration` (feature `chrono`).",
        "",
        "## Higher layers",
        "",
        "| Item | What it does | Feature |",
        "|---|---|---|",
    ]
    for file, item, what, feat in m.layers:
        L.append(f"| [`{item}`](../src/{file}) | {what} | {('`' + feat + '`') if feat else 'always'} |")
    L += ["", "## Not implemented", ""]
    L += [
        "Every code the vendored standards allocate that `dvb-si` does not type yet (generated from",
        "the rows above). Contributions follow `docs/CRATE-ACCEPTANCE.md`: spec transcription and a",
        "real fixture first.",
        "",
    ]
    L += not_impl_summary(m)
    L += [
        "",
        "## Citation cross-check",
        "",
        "Every module's `//!` citation is compared with the clause verified against the edition in",
        "the Standard column. The generator (and so the `docs_coverage_drift` test) **fails** if a",
        "module cites a clause that is neither the verified clause nor a sub-clause of it (issue",
        f"#{ISSUES['citations']}); reasoned exceptions live in `CITATION_ALLOWLIST` in",
        "`tools/gen-dvbsi-coverage.py`.",
        "",
    ]
    L += cross_check(m)
    L.append("")
    return "\n".join(L)


def render_block(m: Model, comment: str) -> list[str]:
    lines = [BEGIN, ""] + glance_block(m, "") + ["", END]
    if comment:
        return [(comment + " " + l).rstrip() for l in lines]
    return lines


def splice(text: str, block: list[str], path: Path, comment: str = "", begin: str = BEGIN, end: str = END) -> str:
    b = (comment + " " + begin).strip() if comment else begin
    e = (comment + " " + end).strip() if comment else end
    lines = text.split("\n")
    try:
        s = next(i for i, l in enumerate(lines) if l.strip() == b)
        t = next(i for i, l in enumerate(lines) if l.strip() == e)
    except StopIteration:
        raise GenError(f"{rel(path)}: generated-block markers not found ({begin!r} … {end!r})")
    return "\n".join(lines[:s] + block + lines[t + 1 :])


def outputs(m: Model) -> dict[Path, str]:
    return {
        COVERAGE: render_coverage(m),
        README: splice(read(README), render_block(m, ""), README),
        LIB: splice(read(LIB), render_block(m, "//!"), LIB, "//!"),
        DOCS_INDEX: splice(read(DOCS_INDEX), render_docs_index(), DOCS_INDEX, begin=INDEX_BEGIN, end=INDEX_END),
    }


def main(argv: list[str]) -> int:
    check = "--check" in argv[1:]
    try:
        m = build()
        bad = citation_mismatches(m)
        if bad:
            raise GenError(
                "module `//!` citations disagree with the verified clause in allocations.tsv (issue #1287):\n"
                + "\n".join(
                    f"  {rel(i.module)}: cites {', '.join(sorted(n))}, verified {a.clause} ({a.standard})"
                    for a, i, n in bad
                )
                + "\nFix the module doc: cite the clause that defines the structure, with the edition."
            )
        unv = unverified_rows(m)
        if unv:
            raise GenError(
                "implemented rows without a verified_page in allocations.tsv (render the PDF page, read the "
                "clause heading + syntax, then record the PDF page number):\n"
                + "\n".join(f"  {a.namespace} {fmt_code(a.lo, a.hi)} {a.standard} §{a.clause}" for a, _ in unv)
            )
        outs = outputs(m)
    except GenError as e:
        print(f"gen-dvbsi-coverage: {e}", file=sys.stderr)
        return 2
    stale = []
    for path, new in outs.items():
        old = path.read_text(encoding="utf-8") if path.exists() else ""
        if old == new:
            continue
        if check:
            stale.append(path)
            diff = difflib.unified_diff(
                old.splitlines(), new.splitlines(), f"{rel(path)} (committed)", f"{rel(path)} (generated)", lineterm="", n=1
            )
            print("\n".join(list(diff)[:80]), file=sys.stderr)
        else:
            path.write_text(new, encoding="utf-8")
            print(f"wrote {rel(path)}")
    if stale:
        print(
            "\ngen-dvbsi-coverage: stale generated docs: "
            + ", ".join(rel(p) for p in stale)
            + "\nRun `python3 tools/gen-dvbsi-coverage.py` and commit the result.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
