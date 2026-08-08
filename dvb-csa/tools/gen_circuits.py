#!/usr/bin/env python3
"""Generate the bitsliced boolean circuits for dvb-csa's `bitsliced` feature.

Reads the cipher tables out of `src/tables.rs` (the single source of truth) and
emits `src/bitsliced/circuits.rs` — straight-line, branch-free Rust that
evaluates each table as a boolean circuit over `Word` (u64) lanes.

Usage (from the crate root):

    python3 tools/gen_circuits.py . && cargo fmt -p dvb-csa

The emitted Rust is valid but unformatted; rustfmt makes it match the
committed file, and `cargo fmt --all --check` is a CI gate.

Two synthesis strategies are tried per circuit and the cheaper one wins:

  * **ROBDD** — a reduced ordered binary decision diagram shared across all
    output bits of the table; each node becomes one bitwise multiplexer.
  * **ANF** — algebraic normal form with shared monomial products.

Every emitted circuit is exhaustively verified against its source table by the
tests in `src/bitsliced/circuits.rs`, so a generator bug cannot ship.
"""

import itertools
import random
import re
import sys

# --------------------------------------------------------------------------
# table extraction
# --------------------------------------------------------------------------


def load_tables(path):
    src = open(path).read()

    def grab(decl):
        i = src.index(decl)
        j = src.index("[", src.index("=", i))
        k = src.index("];", j)
        return [int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]+)", src[j:k])]

    sbox = grab("const SBOX: [u8; 256]")
    perm = grab("const PERM: [u8; 256]")
    ssb = grab("const STREAM_SBOX: [[u16; 32]; 7]")
    cdef = grab("const STREAM_CDEF: [u16; 0x400]")
    out = grab("const STREAM_OUT: [u8; 16]")
    assert len(sbox) == 256 and len(perm) == 256
    assert len(ssb) == 224 and len(cdef) == 1024 and len(out) == 16
    return sbox, perm, [ssb[n * 32 : (n + 1) * 32] for n in range(7)], cdef, out


# --------------------------------------------------------------------------
# circuit synthesis
# --------------------------------------------------------------------------


class Circuit:
    """A straight-line bitwise circuit under construction."""

    def __init__(self, invars):
        self.invars = invars  # list of Rust expressions, one per input bit
        self.lines = []
        self.gates = 0

    def emit(self, expr):
        n = "t%d" % len(self.lines)
        self.lines.append("    let %s = %s;" % (n, expr))
        self.gates += 1
        return n


def _cofactor_masks(nvars):
    full = (1 << (1 << nvars)) - 1
    vmask = []
    for v in range(nvars):
        m = 0
        for x in range(1 << nvars):
            if x & (1 << v):
                m |= 1 << x
        vmask.append(m)
    return full, vmask


def bdd_cost(funcs, nvars, order):
    """Gate cost of the shared ROBDD for `funcs` under `order` (no emission)."""
    full, vmask = _cofactor_masks(nvars)

    def cof(tt, v):
        m = vmask[v]
        step = 1 << v
        lo = tt & ~m
        hi = tt & m
        return ((lo | (lo << step)) & full, (hi | (hi >> step)) & full)

    memo = {}
    cost = [0]

    def rec(tt, depth):
        if tt == 0:
            return "0"
        if tt == full:
            return "1"
        key = (tt, depth)
        if key in memo:
            return memo[key]
        v = order[depth]
        lo, hi = cof(tt, v)
        if lo == hi:
            r = rec(lo, depth + 1)
        else:
            a = rec(lo, depth + 1)
            b = rec(hi, depth + 1)
            cost[0] += 1 if (a in "01" or b in "01") else 3
            r = "n%d" % cost[0]
        memo[key] = r
        return r

    for f in funcs:
        rec(f, 0)
    return cost[0]


def bdd_build(circ, funcs, nvars, order):
    """Emit the shared ROBDD for `funcs` into `circ`; return output expressions."""
    full, vmask = _cofactor_masks(nvars)

    def cof(tt, v):
        m = vmask[v]
        step = 1 << v
        lo = tt & ~m
        hi = tt & m
        return ((lo | (lo << step)) & full, (hi | (hi >> step)) & full)

    memo = {}

    def rec(tt, depth):
        if tt == 0:
            return "ZERO"
        if tt == full:
            return "ONES"
        key = (tt, depth)
        if key in memo:
            return memo[key]
        v = order[depth]
        lo, hi = cof(tt, v)
        if lo == hi:
            r = rec(lo, depth + 1)
        else:
            a = rec(lo, depth + 1)
            b = rec(hi, depth + 1)
            x = circ.invars[v]
            if a == "ZERO":
                r = circ.emit("%s & %s" % (b, x))
            elif b == "ZERO":
                r = circ.emit("%s & !%s" % (a, x))
            elif a == "ONES":
                r = circ.emit("%s | !%s" % (b, x))
            elif b == "ONES":
                r = circ.emit("%s | %s" % (a, x))
            else:
                r = circ.emit("%s ^ ((%s ^ %s) & %s)" % (a, a, b, x))
        memo[key] = r
        return r

    return [rec(f, 0) for f in funcs]


def anf_monomials(tt, nvars):
    n = 1 << nvars
    a = [(tt >> x) & 1 for x in range(n)]
    for i in range(nvars):
        step = 1 << i
        for j in range(n):
            if j & step:
                a[j] ^= a[j ^ step]
    return [m for m in range(n) if a[m]]


def anf_cost(funcs, nvars):
    """Cost of the ANF circuit — measured by building it into a scratch circuit."""
    scratch = Circuit(["x%d" % v for v in range(nvars)])
    anf_build(scratch, funcs, nvars)
    return scratch.gates


def anf_build(circ, funcs, nvars):
    """Emit an ANF circuit with shared monomial products."""
    need = set()
    per = []
    for f in funcs:
        ms = anf_monomials(f, nvars)
        per.append(ms)
        need |= set(ms)
    have = {1 << v: circ.invars[v] for v in range(nvars)}

    def product(m):
        """Build monomial `m`, creating any sub-products it needs."""
        if m in have:
            return have[m]
        low = m & -m
        have[m] = circ.emit("%s & %s" % (product(m ^ low), have[low]))
        return have[m]

    for m in sorted(need, key=lambda m: bin(m).count("1")):
        if m != 0:
            product(m)
    outs = []
    for ms in per:
        if not ms:
            outs.append("ZERO")
            continue
        terms = [("ONES" if m == 0 else have[m]) for m in ms]
        acc = terms[0]
        for t in terms[1:]:
            acc = circ.emit("%s ^ %s" % (acc, t))
        outs.append(acc)
    return outs


def synth(name, funcs, nvars, invars, order_budget=None):
    """Synthesise the cheapest circuit for `funcs`; return (lines, outs, gates)."""
    orders = list(itertools.permutations(range(nvars)))
    if order_budget is not None and len(orders) > order_budget:
        rng = random.Random(0)
        orders = [tuple(range(nvars))] + [
            tuple(rng.sample(range(nvars), nvars)) for _ in range(order_budget)
        ]
    best_order, best = None, None
    for o in orders:
        c = bdd_cost(funcs, nvars, list(o))
        if best is None or c < best:
            best, best_order = c, o
    acost = anf_cost(funcs, nvars)
    circ = Circuit(invars)
    if acost < best:
        outs = anf_build(circ, funcs, nvars)
        how = "ANF"
    else:
        outs = bdd_build(circ, funcs, nvars, list(best_order))
        how = "ROBDD, order %s" % (list(best_order),)
    print(
        "  %-16s %-28s %4d gates (bdd %d / anf %d)"
        % (name, how, circ.gates, best, acost),
        file=sys.stderr,
    )
    return circ.lines, outs, circ.gates, how


def truth_tables(table, out_bits, in_bits):
    """Column-wise truth tables: one big-int per output bit."""
    fs = []
    for ob in out_bits:
        tt = 0
        for x in range(1 << in_bits):
            if (table[x] >> ob) & 1:
                tt |= 1 << x
        fs.append(tt)
    return fs


# --------------------------------------------------------------------------
# derived structural constants
#
# Everything below is *derived* from the tables, never hand-typed: each helper
# recovers the structure it needs and asserts the property it relies on, so a
# table edit that broke an assumption would fail generation rather than emit a
# silently wrong circuit.
# --------------------------------------------------------------------------


def perm_bit_map(perm):
    """`PERM` is GF(2)-linear and maps single bits to single bits: return p[k].

    `PERM[s]` therefore costs nothing when bitsliced — it is pure rewiring,
    `perm_s[p[k]] = s[k]`.
    """
    for a in range(256):
        for b in range(256):
            assert perm[a] ^ perm[b] == perm[a ^ b], "PERM is not GF(2)-linear"
    p = []
    for k in range(8):
        v = perm[1 << k]
        assert v and (v & (v - 1)) == 0, "PERM does not map bit %d to one bit" % k
        p.append(v.bit_length() - 1)
    assert sorted(p) == list(range(8)), "PERM is not a bit permutation"
    return p


def stream_sbox_out_bits(streamsbox):
    """Which two `pqzyx` bit positions each stream S-box drives."""
    res = []
    for t in streamsbox:
        bits = sorted({k for v in t for k in range(16) if (v >> k) & 1})
        assert len(bits) == 2, "stream S-box does not drive exactly 2 bits"
        res.append(bits)
    return res


def parse_sbox_sel(path):
    """Parse `STREAM_SBOX_SEL` out of tables.rs."""
    src = open(path).read()
    i = src.index("const STREAM_SBOX_SEL:")
    j = src.index("[", src.index("=", i))
    k = src.index("];", j)
    body = src[j:k]
    out = []
    for m in re.finditer(r"\(\s*(0x[0-9a-fA-F]+)\s*,\s*(\d+)\s*,\s*\[([^\]]*)\]", body):
        mask = int(m.group(1), 16)
        idx = int(m.group(2))
        shifts = [int(x) for x in m.group(3).split(",")]
        out.append((mask, idx, shifts))
    assert len(out) == 7, "expected 7 STREAM_SBOX_SEL entries, got %d" % len(out)
    return out


def stream_sbox_input_map(sel, a_bits):
    """Recover each stream S-box's five index bits as XORs of A-register bits.

    `csa_stream_sboxes` computes, per S-box,
    `idx = ((t >> s0) ^ (t >> s1) ^ ... ) & 0x1f` where `t = a & mask`.
    That is GF(2)-linear in `a`, so each index bit is the XOR of a fixed set of
    A bits — which is all the bitsliced form needs.
    """
    per_sbox = [None] * 7
    for mask, sbox_index, shifts in sel:
        taps = [[] for _ in range(5)]
        for i in range(a_bits):
            if not (mask >> i) & 1:
                continue
            acc = 0
            for s in shifts:
                acc ^= (1 << i) >> s
            acc &= 0x1F
            for k in range(5):
                if (acc >> k) & 1:
                    taps[k].append(i)
        assert all(taps), "S-box %d has an unfed index bit" % sbox_index
        assert per_sbox[sbox_index] is None, "duplicate S-box index"
        per_sbox[sbox_index] = taps
    assert all(t is not None for t in per_sbox), "STREAM_SBOX_SEL is incomplete"

    # Cross-check the recovered linear map against the scalar routine itself.
    def scalar_idx(a, mask, shifts):
        t = a & mask
        acc = 0
        for s in shifts:
            acc ^= t >> s
        return acc & 0x1F

    rng = random.Random(2)
    for _ in range(20000):
        a = rng.getrandbits(a_bits)
        for mask, sbox_index, shifts in sel:
            want = scalar_idx(a, mask, shifts)
            got = 0
            for k, tap in enumerate(per_sbox[sbox_index]):
                bit = 0
                for i in tap:
                    bit ^= (a >> i) & 1
                got |= bit << k
            assert got == want, "recovered S-box index map disagrees with tables"
    return per_sbox


def b_sel_map(b_bits):
    """`csa_stream_b_sel` is GF(2)-linear in B: recover its four output taps."""

    def b_sel(b):
        t = (b >> 9) & 0xFFFFFFFF  # the C truncation to 32 bits is load-bearing
        return (
            ((t ^ (t >> 27)) & 0x8)
            ^ ((t >> 18) & 0x9)
            ^ (((t >> 22) ^ (t >> 7)) & 0x4)
            ^ ((t >> 4) & 0x5)
            ^ (((t >> 24) ^ (t >> 6) ^ (t >> 11)) & 0x2)
            ^ (((t >> 29) ^ (t >> 23)) & 0x1)
            ^ ((t >> 13) & 0xE)
        )

    assert b_sel(0) == 0, "csa_stream_b_sel is not homogeneous"
    basis = [b_sel(1 << i) for i in range(b_bits)]
    rng = random.Random(3)
    for _ in range(20000):
        v = rng.getrandbits(b_bits)
        acc = 0
        for i in range(b_bits):
            if (v >> i) & 1:
                acc ^= basis[i]
        assert acc == b_sel(v), "csa_stream_b_sel is not GF(2)-linear"
    taps = [[i for i in range(b_bits) if (basis[i] >> k) & 1] for k in range(4)]
    assert all(taps), "csa_stream_b_sel has an unfed output bit"
    return taps


def stream_out_map(out):
    """`STREAM_OUT[d]` is a 2-bit value replicated four times; recover both bits.

    Each is GF(2)-linear in the D nibble, so the keystream costs two XORs.
    """
    for d in range(16):
        v = out[d]
        assert v == ((v >> 6) & 3) * 0b01010101, "STREAM_OUT[%d] is not replicated" % d
    taps = []
    for shift in (7, 6):  # high bit of the pair, then the low bit
        f = sum(((out[d] >> shift) & 1) << d for d in range(16))
        ms = anf_monomials(f, 4)
        assert ms and all(bin(m).count("1") == 1 for m in ms), (
            "STREAM_OUT bit is not linear in the D nibble"
        )
        taps.append(sorted(m.bit_length() - 1 for m in ms))
    return taps


# --------------------------------------------------------------------------
# emission
# --------------------------------------------------------------------------

HEADER = '''//! Bitsliced boolean circuits for DVB-CSA2 — **generated code, do not edit**.
//!
//! Regenerate with `python3 tools/gen_circuits.py . && cargo fmt -p dvb-csa`
//! from the `dvb-csa` crate root (the generator emits valid but unformatted
//! Rust; rustfmt makes it match the committed file). It reads [`crate::tables`] — which stays the single source
//! of truth for the cipher — and re-expresses each table as a straight-line,
//! branch-free circuit over [`Word`](super::Word) lanes, so one evaluation
//! covers [`LANES`](super::LANES) independent blocks at once.
//!
//! Two syntheses are tried per table and the cheaper wins: a shared ROBDD (one
//! bitwise multiplexer per node) or an algebraic normal form with shared
//! monomials. The linear tables (`PERM`, the S-box index selection,
//! `csa_stream_b_sel`, `STREAM_OUT`) reduce to rewiring plus a few XORs, and
//! the generator asserts that linearity rather than assuming it.
//!
//! **Nothing here is taken on trust.** `circuit_tests.rs` — deliberately not
//! generated, so regeneration cannot regenerate its own gate — evaluates every
//! circuit over its *entire* input domain and compares against the table it was
//! generated from. A generator bug cannot ship.
#![allow(clippy::identity_op)]

use super::Word;

/// All-zero lane mask — used only where a table column is identically zero.
#[allow(dead_code)]
const ZERO: Word = 0;
/// All-ones lane mask — used only where a table column is identically one.
#[allow(dead_code)]
const ONES: Word = !0;

'''


def emit_fn(doc, sig, lines, ret):
    return "%s#[inline]\n%s\n%s\n    %s\n}\n\n" % (doc, sig, "\n".join(lines), ret)


def xor_expr(arr, taps):
    return " ^ ".join("%s[%d]" % (arr, i) for i in taps)


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    tables = root + "/src/tables.rs"
    sbox, perm, ssb, cdef, sout = load_tables(tables)
    sel = parse_sbox_sel(tables)

    a_bits, b_bits = 40, 40
    print("synthesising circuits:", file=sys.stderr)
    out = [HEADER]
    budget = [0]

    def note(g):
        budget[0] += g

    # ---- block S-box: 8 -> 8 ------------------------------------------------
    invars = ["x%d" % k for k in range(8)]
    fs = truth_tables(sbox, range(8), 8)
    lines, outs, gates, how = synth("block_sbox", fs, 8, invars)
    doc = (
        "/// Bitsliced `SBOX[..]` — the block cipher's 8-bit substitution box.\n"
        "///\n"
        "/// `x[k]` carries bit `k` of the S-box input for every lane; the\n"
        "/// result carries bit `k` of `SBOX[input]` for every lane.\n"
        "///\n"
        "/// Synthesis: %s — %d gates.\n" % (how, gates)
    )
    sig = "pub(super) fn block_sbox(x: &[Word; 8]) -> [Word; 8] {"
    pre = ["    let [%s] = *x;" % ", ".join(invars)]
    out.append(emit_fn(doc, sig, pre + lines, "[%s]" % ", ".join(outs)))

    # ---- stream S-boxes: 5 -> 2 each ---------------------------------------
    sb_out = stream_sbox_out_bits(ssb)
    stotal = 0
    for n in range(7):
        invars = ["x%d" % k for k in range(5)]
        fs = truth_tables(ssb[n], sb_out[n], 5)
        lines, outs, gates, how = synth("stream_sbox_%d" % n, fs, 5, invars)
        stotal += gates
        doc = (
            "/// Bitsliced `STREAM_SBOX[%d]` — five A-register index bits in,\n"
            "/// the `pqzyx` bits at positions %d and %d out.\n"
            "///\n"
            "/// Synthesis: %s — %d gates.\n"
            % (n, sb_out[n][0], sb_out[n][1], how, gates)
        )
        sig = "fn stream_sbox_%d(x: &[Word; 5]) -> [Word; 2] {" % n
        pre = ["    let [%s] = *x;" % ", ".join(invars)]
        out.append(emit_fn(doc, sig, pre + lines, "[%s]" % ", ".join(outs)))
    print("  %-16s %-28s %4d gates" % ("(7 sboxes)", "total", stotal), file=sys.stderr)

    # ---- STREAM_CDEF: 10 -> 9 ----------------------------------------------
    cbits = sorted({k for v in cdef for k in range(16) if (v >> k) & 1})
    invars = ["x%d" % k for k in range(10)]
    fs = truth_tables(cdef, cbits, 10)
    lines, outs, gates, how = synth("stream_cdef", fs, 10, invars, order_budget=4000)
    doc = (
        "/// Bitsliced `STREAM_CDEF[..]` — the C/D/E/F feedback table, ten index\n"
        "/// bits in, its nine live output bits out (positions\n"
        "/// [`CDEF_OUT_BITS`]); every other position of the table is zero.\n"
        "///\n"
        "/// Synthesis: %s — %d gates.\n" % (how, gates)
    )
    sig = "pub(super) fn stream_cdef(x: &[Word; 10]) -> [Word; 9] {"
    pre = ["    let [%s] = *x;" % ", ".join(invars)]
    out.append(emit_fn(doc, sig, pre + lines, "[%s]" % ", ".join(outs)))

    # ---- linear structure ---------------------------------------------------
    p = perm_bit_map(perm)
    smap = stream_sbox_input_map(sel, a_bits)
    bsel = b_sel_map(b_bits)
    otaps = stream_out_map(sout)

    out.append(
        "/// Bit destinations of the block cipher's `PERM` table.\n"
        "///\n"
        "/// `PERM` is GF(2)-linear and maps each input bit to exactly one output\n"
        "/// bit, so bitsliced it is free: `PERM[s]` bit `PERM_BIT[k]` is `s` bit\n"
        "/// `k`.\n"
        "pub(super) const PERM_BIT: [usize; 8] = %s;\n\n" % (list(p),)
    )
    out.append(
        "/// `pqzyx` bit positions written by [`stream_sboxes`], in the order it\n"
        "/// returns them.\n"
        "pub(super) const STREAM_SBOX_OUT_BITS: [usize; 14] = %s;\n\n"
        % ([b for pair in sb_out for b in pair],)
    )
    out.append(
        "/// `cfed` bit positions written by [`stream_cdef`], in the order it\n"
        "/// returns them.\n"
        "pub(super) const CDEF_OUT_BITS: [usize; 9] = %s;\n\n" % (list(cbits),)
    )

    # stream_sboxes: select inputs from A, then run the seven S-boxes
    lines = []
    for n in range(7):
        for k in range(5):
            lines.append(
                "    let s%d_%d = %s;" % (n, k, xor_expr("a", smap[n][k]))
            )
    for n in range(7):
        lines.append(
            "    let o%d = stream_sbox_%d(&[%s]);"
            % (n, n, ", ".join("s%d_%d" % (n, k) for k in range(5)))
        )
    ret = "[%s]" % ", ".join("o%d[0], o%d[1]" % (n, n) for n in range(7))
    doc = (
        "/// Bitsliced `csa_stream_sboxes` — the A register in, the fourteen\n"
        "/// `pqzyx` bits out, ordered as [`STREAM_SBOX_OUT_BITS`].\n"
        "///\n"
        "/// The index of each S-box is a GF(2)-linear function of the A bits\n"
        "/// (`STREAM_SBOX_SEL`'s mask-and-shift chain), so selecting it costs\n"
        "/// only XORs.\n"
    )
    sig = "pub(super) fn stream_sboxes(a: &[Word; %d]) -> [Word; 14] {" % a_bits
    out.append(emit_fn(doc, sig, lines, ret))

    doc = (
        "/// Bitsliced `csa_stream_b_sel` — four bits XOR-folded out of the B\n"
        "/// register. Linear, so it is pure XOR.\n"
    )
    sig = "pub(super) fn stream_b_sel(b: &[Word; %d]) -> [Word; 4] {" % b_bits
    lines = ["    let o%d = %s;" % (k, xor_expr("b", bsel[k])) for k in range(4)]
    out.append(emit_fn(doc, sig, lines, "[o0, o1, o2, o3]"))

    doc = (
        "/// Bitsliced `STREAM_OUT[d]` — the two keystream bits a round yields.\n"
        "///\n"
        "/// Every entry of `STREAM_OUT` is a 2-bit value replicated across the\n"
        "/// byte, and each of those bits is linear in the D nibble, so a round's\n"
        "/// keystream contribution is two XORs. Returns `[high, low]`.\n"
    )
    sig = "pub(super) fn stream_out(d: &[Word; 4]) -> [Word; 2] {"
    lines = [
        "    let hi = %s;" % xor_expr("d", otaps[0]),
        "    let lo = %s;" % xor_expr("d", otaps[1]),
    ]
    out.append(emit_fn(doc, sig, lines, "[hi, lo]"))

    dst = root + "/src/bitsliced/circuits.rs"
    open(dst, "w").write("".join(out))
    print("\nPERM bit map          : %s" % (p,), file=sys.stderr)
    print("STREAM_OUT taps       : %s" % (otaps,), file=sys.stderr)
    print("cdef live output bits : %s" % (cbits,), file=sys.stderr)
    print("wrote %s" % dst, file=sys.stderr)


main()
