#!/usr/bin/env python3
"""dhat growth-diff for the spec371 memory diagnosis (python3 stdlib only).

usage: spec371-dhat-diff.py <early.dhat.json.gz> <late.dhat.json.gz>

Both profiles come from the SAME dhat build, so their frame strings compare
directly. Allocator frames are dropped from the top of every stack; the next
five frames, joined, are the callsite signature. eb (bytes live at the end of
the run), ebk and gb (bytes live at the global peak) are aggregated per
signature, and growth is eb_late - eb_early. The top grower is mapped to a
lever through a fixed token table. Printed as plain text on stdout.

Known depth limit: the server's dhat Profiler keeps the crate default of ten
frames per backtrace, so a signature can never reach below frame ten of the
allocation stack.
"""
import gzip
import json
import re
import sys

# Leading frames that are the allocator or the standard library: stripped from
# the top of every stack so a signature starts at the first frame this codebase
# owns. Without the std prefixes the five kept frames are container internals
# (Arc::allocate_for_slice, Vec::push, slice::to_vec) and the token table cannot
# see the holder at all.
ALLOC_FRAME = re.compile(
    r"^\[root\]|^dhat::|^__rust_|^alloc::|^core::|^std::|^<alloc::|^<core::|^<std::|"
    r"^<T as alloc::|^<T as core::"
)
ADDR = re.compile(r"^0x[0-9a-fA-F]+: ")
UNSYMBOLISED = re.compile(r"\?\?\?:0:0|__mh_execute_header")
SIG_FRAMES = 5
# Tried in LIST ORDER over the joined signature string; the first token that
# occurs anywhere in it wins. Frame order does not matter.
LEVERS = [
    ("591", re.compile(r"WriteBehindDataStore")),
    ("JOURNAL", re.compile(r"record_journal|JournalStore")),
    ("593", re.compile(r"apply_or_delta|update_in_place|merge_add")),
    ("592", re.compile(r"redb::")),
    ("588", re.compile(r"broadcast|ServerEventPayload|or_record")),
]


def load(path):
    with gzip.open(path, "rt") as fh:
        return json.load(fh)


def frames(prof, pp):
    out = []
    for i in pp.get("fs", []):
        out.append(ADDR.sub("", prof["ftbl"][i]))
    return out


def signature(fr):
    """The first SIG_FRAMES frames after the leading allocator/std frames.

    Fallback: when stripping leaves nothing (dhat's 10-frame trim ended inside
    std), the signature is the literal ALL_STD: plus the first three raw frames.
    Its lever is UNMAPPED, and the caller reports how much growth such stacks
    hold, so a deep-std top grower cannot route a REACHABLE result to a ruling
    by construction.
    """
    k = 0
    while k < len(fr) and ALLOC_FRAME.search(fr[k]):
        k += 1
    kept = fr[k:k + SIG_FRAMES]
    if not kept:
        return "ALL_STD: " + " <- ".join(fr[:3])
    return " <- ".join(kept)


def aggregate(prof):
    agg = {}
    for pp in prof["pps"]:
        sig = signature(frames(prof, pp))
        a = agg.setdefault(sig, {"eb": 0, "ebk": 0, "gb": 0})
        a["eb"] += pp.get("eb", 0)
        a["ebk"] += pp.get("ebk", 0)
        a["gb"] += pp.get("gb", 0)
    return agg


def lever(sig):
    if sig.startswith("ALL_STD:"):
        return "UNMAPPED"
    for name, rx in LEVERS:
        if rx.search(sig):
            return name
    return "UNMAPPED"


def mb(b):
    return b / 1048576.0


def main():
    if len(sys.argv) != 3:
        print("usage: spec371-dhat-diff.py <early.dhat.json.gz> <late.dhat.json.gz>", file=sys.stderr)
        return 2
    try:
        early, late = load(sys.argv[1]), load(sys.argv[2])
    except (OSError, ValueError) as exc:
        print(f"PD-format=FALSE reason=unreadable {exc}")
        print("PD-sym=FALSE reason=unreadable")
        print("PD-crate=FALSE reason=unreadable")
        print("C3-top1-lever=UNMAPPED reason=unreadable")
        return 0

    fmt_ok = True
    for name, prof in (("early", early), ("late", late)):
        ver, mode = prof.get("dhatFileVersion"), prof.get("mode")
        eb = sum(pp.get("eb", 0) for pp in prof.get("pps", []))
        gb = sum(pp.get("gb", 0) for pp in prof.get("pps", []))
        print(f"total {name}: dhatFileVersion={ver} mode={mode} te={prof.get('te')} "
              f"sum_eb={eb} ({mb(eb):.2f} MB) sum_gb={gb} ({mb(gb):.2f} MB) pps={len(prof.get('pps', []))}")
        if ver != 2 or mode != "rust-heap" or eb <= 0:
            fmt_ok = False

    ea, la = aggregate(early), aggregate(late)
    rows = []
    for sig in set(ea) | set(la):
        e = ea.get(sig, {"eb": 0, "ebk": 0, "gb": 0})
        l = la.get(sig, {"eb": 0, "ebk": 0, "gb": 0})
        rows.append((l["eb"] - e["eb"], l["ebk"] - e["ebk"], e["eb"], l["eb"], sig))
    rows.sort(key=lambda r: r[0], reverse=True)

    print()
    print("top-10 by growth (delta eb = eb_late - eb_early):")
    print("rank  d_eb_MB  d_ebk  eb_early_MB  eb_late_MB  signature")
    for i, (d, dk, e, l, sig) in enumerate(rows[:10], 1):
        print(f"{i:>4}  {mb(d):>7.2f}  {dk:>5}  {mb(e):>11.2f}  {mb(l):>10.2f}  {sig}")

    print()
    print("top-10 by eb_late (END snapshot):")
    for i, (sig, a) in enumerate(sorted(la.items(), key=lambda kv: kv[1]["eb"], reverse=True)[:10], 1):
        print(f"{i:>4}  {mb(a['eb']):>8.2f} MB  {a['ebk']:>7} blocks  {sig}")

    # Symbolisation: of the top-50 program points by eb in the LATE profile, at
    # least 80 % must carry no unsymbolised-build frame.
    top50 = sorted(late["pps"], key=lambda pp: pp.get("eb", 0), reverse=True)[:50]
    clean = sum(1 for pp in top50 if not any(UNSYMBOLISED.search(f) for f in frames(late, pp)))
    sym_ok = bool(top50) and clean >= 0.8 * len(top50)
    crate_ok = any("topgun_server" in r[4] for r in rows[:10])

    # ALL_STD accounting.
    all_std = [r for r in rows if r[4].startswith("ALL_STD:")]
    pos_top = sum(r[0] for r in rows[:10] if r[0] > 0) or 1
    all_std_share = sum(r[0] for r in all_std[:10] if r[0] > 0) / pos_top

    # lever_share: positive growth per lever token over the top-10 growers.
    share = {}
    for r in rows[:10]:
        if r[0] > 0:
            share[lever(r[4])] = share.get(lever(r[4]), 0) + r[0]
    ranked = sorted(share.items(), key=lambda kv: kv[1], reverse=True)

    # LEVER stays top-1 (pre-registered). When the top-1 signature is ALL_STD the
    # lever is taken from the first non-ALL_STD signature instead.
    top1_all_std = bool(rows) and rows[0][4].startswith("ALL_STD:")
    pick = next((r for r in rows if not r[4].startswith("ALL_STD:")), None) if top1_all_std else (rows[0] if rows else None)
    top1_lever = lever(pick[4]) if pick else "UNMAPPED"

    print()
    print("top-3 levers: " + ", ".join(f"{i}:{lever(r[4])}" for i, r in enumerate(rows[:3], 1)))
    print("lever_share: " + ", ".join(f"{k}={mb(v):.2f}MB({v / pos_top:.0%})" for k, v in ranked))
    print(f"all_std_pps={len(all_std)} all_std_share={all_std_share:.0%}")
    print(f"C3-top1-all-std={'TRUE' if top1_all_std else 'FALSE'}")
    print(f"LEVER_CONTESTED={'TRUE' if ranked and ranked[0][0] != top1_lever else 'FALSE'}")
    print(f"PD-format={'TRUE' if fmt_ok else 'FALSE'}")
    print(f"PD-sym={'TRUE' if sym_ok else 'FALSE'} symbolised={clean}/{len(top50)}")
    print(f"PD-crate={'TRUE' if crate_ok else 'FALSE'}")
    print(f"C3-top1-lever={top1_lever}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
