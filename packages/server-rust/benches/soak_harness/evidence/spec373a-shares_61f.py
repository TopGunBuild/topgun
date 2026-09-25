#!/usr/bin/env python3
"""SPEC-373a E program: removable leaf-hash share + prune-probe share of a dhat window.

usage (LC_ALL=C):
  shares_61f.py --pin 46dcc12a C3E C3L
      the self-check over the committed spec371-c3{e,l} pair (known answer)
  shares_61f.py --pin 61f84658 C3E C3L --old C3E_371 C3L_371
      the pin reading; --old re-runs the 46dcc12a self-check over the 371 pair
      in-program and takes its OUTPUT shares as the STOP-S "old" references

Method (fixed in SPEC-373a, Measurement):
  window = c3l - c3e (total_bytes, dhat 'tb'); every share = bytes / window total.
  LEAF_87/90/93: bytes of program points whose FIRST (innermost) topgun_* frame --
    a frame containing 'topgun_server::' or 'topgun_core::' -- sits at
    storage/map_data_store.rs:87 / :90 / :93. LEAF_SHARE = their sum / window.
  PRUNE: bytes of every program point whose chain contains BOTH the engine-get-clone
    frame and the prune probe site (pin-selected literals below), summed in-program,
    no top-N cut. PRUNE_SHARE = PRUNE / window.
  OVERLAP: bytes of program points satisfying both the LEAF and the PRUNE predicate.
Output: one KEY=value per line; STOP before the frozen prediction values.
STOP precedence D > O > S; the first that fires is printed.
"""
import gzip
import json
import re
import sys

# Literals from the code at each pin. Derived by (see manifest section 1):
#   git show <pin>:packages/server-rust/src/storage/map_data_store.rs | grep -n 'tags.join\|tomb_tags.join\|fnv1a_hash(&format!("key:'
#   git show <pin>:packages/server-rust/src/storage/engines/hashmap.rs | grep -n 'fn get(' -A1
#   git show <pin>:packages/server-rust/src/service/domain/crdt.rs | grep -n 'match store.get(&r.key, false).await'
PINS = {
    "46dcc12a": {
        "leaf": {87: "storage/map_data_store.rs:87", 90: "storage/map_data_store.rs:90",
                 93: "storage/map_data_store.rs:93"},
        "clone": r"storage/engines/hashmap\.rs:6[3-4]:",
        "probe": r"service/domain/crdt\.rs:1780:",
    },
    "61f84658": {
        "leaf": {87: "storage/map_data_store.rs:87", 90: "storage/map_data_store.rs:90",
                 93: "storage/map_data_store.rs:93"},
        "clone": r"storage/engines/hashmap\.rs:92:",
        "probe": r"service/domain/crdt\.rs:1786:",
    },
}

# Self-check known answer (conductor rulings v3 item 6): fractions, tolerance = rounding.
SELF_LEAF = 0.104772
SELF_PRUNE = 0.028618
SELF_TOL = 0.000005
MB = float(2 ** 20)
FRAME_LOC = re.compile(r"\(([^()]*\.rs:\d+):\d+\)$")


def first_topgun_loc(frames):
    for f in frames:
        if "topgun_server::" in f or "topgun_core::" in f:
            m = FRAME_LOC.search(f)
            return m.group(1) if m else None
    return None


def read(path, pin):
    d = json.load(gzip.open(path))
    ftbl = d["ftbl"]
    pat = PINS[pin]
    clone_rx = re.compile(pat["clone"])
    probe_rx = re.compile(pat["probe"])
    out = {"total": 0, "prune": 0, "overlap": 0}
    for line in pat["leaf"]:
        out[line] = 0
    for p in d["pps"]:
        tb = p["tb"]
        frames = [ftbl[i] for i in p["fs"]]
        out["total"] += tb
        loc = first_topgun_loc(frames)
        leaf_line = None
        if loc is not None:
            for line, suffix in pat["leaf"].items():
                if loc.endswith(suffix):
                    leaf_line = line
        if leaf_line is not None:
            out[leaf_line] += tb
        is_prune = any(clone_rx.search(f) for f in frames) and any(probe_rx.search(f) for f in frames)
        if is_prune:
            out["prune"] += tb
        if is_prune and leaf_line is not None:
            out["overlap"] += tb
    return out


def window(c3e, c3l, pin):
    e, l = read(c3e, pin), read(c3l, pin)
    w = {k: l[k] - e[k] for k in l}
    wt = w["total"]
    leaf = w[87] + w[90] + w[93]
    return {
        "WINDOW_MB": wt / MB,
        "LEAF_87_MB": w[87] / MB,
        "LEAF_90_MB": w[90] / MB,
        "LEAF_93_MB": w[93] / MB,
        "LEAF_SHARE": leaf / wt,
        "PRUNE_MB": w["prune"] / MB,
        "PRUNE_SHARE": w["prune"] / wt,
        "OVERLAP_MB": w["overlap"] / MB,
    }


def zero_bucket(r):
    return any(r[k] <= 0 for k in ("LEAF_87_MB", "LEAF_90_MB", "LEAF_93_MB", "PRUNE_MB"))


def emit(r):
    e = r["LEAF_SHARE"] + r["PRUNE_SHARE"]
    for k in ("WINDOW_MB", "LEAF_87_MB", "LEAF_90_MB", "LEAF_93_MB"):
        print("%s=%.4f" % (k, r[k]))
    print("LEAF_SHARE=%.6f" % r["LEAF_SHARE"])
    print("PRUNE_MB=%.4f" % r["PRUNE_MB"])
    print("PRUNE_SHARE=%.6f" % r["PRUNE_SHARE"])
    print("OVERLAP_MB=%.4f" % r["OVERLAP_MB"])
    print("E=%.6f" % e)
    print("P_BYTES=%.6f" % (1 - e))
    return e


def self_check_ok(r):
    return abs(r["LEAF_SHARE"] - SELF_LEAF) <= SELF_TOL and abs(r["PRUNE_SHARE"] - SELF_PRUNE) <= SELF_TOL


def main(argv):
    args = argv[1:]
    try:
        pin = args[args.index("--pin") + 1]
    except (ValueError, IndexError):
        sys.exit("usage: shares_61f.py --pin 46dcc12a|61f84658 C3E C3L [--old C3E_371 C3L_371]")
    if pin not in PINS:
        sys.exit("unknown pin %s" % pin)
    old = None
    if "--old" in args:
        i = args.index("--old")
        old = args[i + 1:i + 3]
        del args[i:i + 3]
    del args[args.index("--pin"):args.index("--pin") + 2]
    if len(args) != 2:
        sys.exit("need exactly C3E C3L")
    c3e, c3l = args
    print("PIN=%s" % pin)
    print("C3E=%s" % c3e)
    print("C3L=%s" % c3l)

    stops = set()
    if pin == "46dcc12a":
        r = window(c3e, c3l, pin)
        e = emit(r)
        print("SELF_CHECK=%s (LEAF %.6f vs %.6f, PRUNE %.6f vs %.6f, tol %.6f)" % (
            "PASS" if self_check_ok(r) else "FAIL", r["LEAF_SHARE"], SELF_LEAF,
            r["PRUNE_SHARE"], SELF_PRUNE, SELF_TOL))
        if not self_check_ok(r) or zero_bucket(r):
            stops.add("D")
        if r["OVERLAP_MB"] > 0:
            stops.add("O")
    else:
        if old is None or len(old) != 2:
            sys.exit("--pin 61f84658 needs --old C3E_371 C3L_371")
        ref = window(old[0], old[1], "46dcc12a")
        print("OLD_C3E=%s" % old[0])
        print("OLD_C3L=%s" % old[1])
        print("OLD_LEAF_SHARE=%.6f" % ref["LEAF_SHARE"])
        print("OLD_PRUNE_SHARE=%.6f" % ref["PRUNE_SHARE"])
        print("OLD_SELF_CHECK=%s" % ("PASS" if self_check_ok(ref) else "FAIL"))
        if not self_check_ok(ref) or zero_bucket(ref):
            stops.add("D")
        if ref["OVERLAP_MB"] > 0:
            stops.add("O")
        r = window(c3e, c3l, pin)
        e = emit(r)
        if zero_bucket(r):
            stops.add("D")
        if r["OVERLAP_MB"] > 0:
            stops.add("O")
        dl = abs(r["LEAF_SHARE"] - ref["LEAF_SHARE"]) / ref["LEAF_SHARE"]
        dp = abs(r["PRUNE_SHARE"] - ref["PRUNE_SHARE"]) / ref["PRUNE_SHARE"]
        print("S_LEAF_REL=%.6f" % dl)
        print("S_PRUNE_REL=%.6f" % dp)
        if dl > 1.0 / 3 or dp > 1.0 / 3:
            stops.add("S")

    stop = next((s for s in ("D", "O", "S") if s in stops), "none")
    print("STOP=%s" % stop)
    if stop == "none":
        print("E_FROZEN=%.4f" % e)
        print("P_BYTES_FROZEN=%.4f" % (1 - e))
    else:
        print("E_FROZEN=WITHHELD")
        print("P_BYTES_FROZEN=WITHHELD")


if __name__ == "__main__":
    main(sys.argv)
