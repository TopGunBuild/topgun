#!/usr/bin/env python3
"""SPEC-373b E program: the share of a dhat window allocated by the three per-op
whole-record copies on the OR write path (engine, queue, staging).

usage (LC_ALL=C):
  spec373b-shares.py --pin 46dcc12a C3E C3L
      the self-check alone, over the committed spec371-c3{e,l} pair
  spec373b-shares.py --pin b166719d DE DL --self C3E_371 C3L_371
      the base reading; --self runs the 46dcc12a self-check over the 371 pair
      IN-PROGRAM, BEFORE the base pair is read, and a miss is STOP-S

Method (SPEC-373b Measurement, "E"):
  window = <late profile> - <early profile>, dhat 'tb' (total bytes allocated);
  every share = bytes / window total.
  Attribution = CHAIN-CONTAINS per copy site: a program point counts for a site
  when ANY frame of its stack sits at that site's line (the innermost topgun
  frame of every RecordValue::clone() is the derived Clone impl, never a call
  site, so an innermost-frame rule cannot see the copies).
  E = (engine + queue + staging) / window.
STOP rules (precedence D > O > S; the first that fires is printed):
  D -- a site line matches no program point in either profile of a pair, or a
       site's window bytes are <= 0 (the literal is stale or the site is dead);
  O -- a program point whose stack contains TWO or more sites (the shares would
       double count);
  S -- the self-check misses its known answer: E = 0.322296 +/- 0.000005.
Units check (recorded, not a STOP): per site, the window bytes of program points
whose stack also holds a realloc / Vec-growth frame. dhat adds the full new size
on every realloc while stats_alloc adds only the growth, so a site with 0 such
bytes is a one-shot allocation whose dhat bytes equal its stats_alloc bytes.
Output: one KEY=value per line; STOP and the frozen value last.
"""
import gzip
import json
import re
import sys

# Literals read from the code at each pin, never transcribed from a spec:
#   git show <pin>:packages/server-rust/src/storage/engines/hashmap.rs | grep -n 'record: record.clone(),'
#   git show <pin>:packages/server-rust/src/storage/datastores/write_behind.rs | grep -n '                value: value.clone(),'
#   git show <pin>:packages/server-rust/src/storage/datastores/write_behind.rs | grep -n 'self.stage(&smap, &skey, entry_seq, Some(value.clone()));'
PINS = {
    "46dcc12a": {
        "engine": "storage/engines/hashmap.rs:107",
        "queue": "storage/datastores/write_behind.rs:2424",
        "staging": "storage/datastores/write_behind.rs:2479",
    },
    "b166719d": {
        "engine": "storage/engines/hashmap.rs:136",
        "queue": "storage/datastores/write_behind.rs:2463",
        "staging": "storage/datastores/write_behind.rs:2518",
    },
}
SITES = ("engine", "queue", "staging")
SELF_PIN = "46dcc12a"
SELF_E = 0.322296
SELF_TOL = 0.000005
MB = float(2 ** 20)
GROW_RX = re.compile(r"finish_grow|realloc")


def site_rx(loc):
    # A dhat frame ends in "(<path>:<line>:<col>)"; anchor the path at "(" and the
    # line at ":" so :136 never matches :1360 and hashmap.rs never matches a
    # longer file name.
    return re.compile(r"\(" + re.escape(loc) + r":\d+\)$")


def read(path, pin):
    d = json.load(gzip.open(path))
    ftbl = d["ftbl"]
    rx = {s: site_rx(PINS[pin][s]) for s in SITES}
    fhit = {s: set(i for i, f in enumerate(ftbl) if rx[s].search(f)) for s in SITES}
    fgrow = set(i for i, f in enumerate(ftbl) if GROW_RX.search(f))
    out = {"total": 0, "multi_pps": 0, "multi": 0}
    for s in SITES:
        out[s] = 0
        out[s + "_pps"] = 0
        out[s + "_grow"] = 0
    for p in d["pps"]:
        tb = p["tb"]
        fs = p["fs"]
        out["total"] += tb
        hit = [s for s in SITES if any(i in fhit[s] for i in fs)]
        if len(hit) > 1:
            out["multi_pps"] += 1
            out["multi"] += tb
        grows = any(i in fgrow for i in fs)
        for s in hit:
            out[s] += tb
            out[s + "_pps"] += 1
            if grows:
                out[s + "_grow"] += tb
    return out


def window(early, late, pin):
    e, l = read(early, pin), read(late, pin)
    w = {k: l[k] - e[k] for k in l}
    wt = w["total"]
    r = {"WINDOW_MB": wt / MB, "MULTI_PPS": e["multi_pps"] + l["multi_pps"]}
    esum = 0
    for s in SITES:
        r["PPS_" + s.upper()] = (e[s + "_pps"], l[s + "_pps"])
        r[s.upper() + "_MB"] = w[s] / MB
        r[s.upper() + "_SHARE"] = w[s] / wt
        r[s.upper() + "_GROW_MB"] = w[s + "_grow"] / MB
        esum += w[s]
    r["E"] = esum / wt
    return r


def emit(prefix, r, pin):
    print("%sPIN=%s" % (prefix, pin))
    print("%sWINDOW_MB=%.4f" % (prefix, r["WINDOW_MB"]))
    for s in SITES:
        u = s.upper()
        print("%s%s_SITE=%s" % (prefix, u, PINS[pin][s]))
        print("%s%s_PPS=%d/%d" % (prefix, u, r["PPS_" + u][0], r["PPS_" + u][1]))
        print("%s%s_MB=%.4f" % (prefix, u, r[u + "_MB"]))
        print("%s%s_SHARE=%.6f" % (prefix, u, r[u + "_SHARE"]))
        print("%s%s_GROW_MB=%.4f" % (prefix, u, r[u + "_GROW_MB"]))
    print("%sMULTI_SITE_PPS=%d" % (prefix, r["MULTI_PPS"]))
    print("%sE=%.6f" % (prefix, r["E"]))
    print("%sE_UNROUNDED=%.7f" % (prefix, r["E"]))


def dead(r):
    for s in SITES:
        u = s.upper()
        if r["PPS_" + u][0] == 0 or r["PPS_" + u][1] == 0 or r[u + "_MB"] <= 0:
            return True
    return False


def usage():
    sys.exit("usage: spec373b-shares.py --pin 46dcc12a C3E C3L\n"
             "       spec373b-shares.py --pin b166719d DE DL --self C3E_371 C3L_371")


def main(argv):
    args = argv[1:]
    try:
        i = args.index("--pin")
        pin = args[i + 1]
        del args[i:i + 2]
    except (ValueError, IndexError):
        usage()
    if pin not in PINS:
        sys.exit("unknown pin %s" % pin)
    ref = None
    if "--self" in args:
        i = args.index("--self")
        ref = args[i + 1:i + 3]
        del args[i:i + 3]
        if len(ref) != 2:
            usage()
    if len(args) != 2:
        usage()
    if pin != SELF_PIN and ref is None:
        sys.exit("--pin %s needs --self C3E_371 C3L_371 (the self-check runs first)" % pin)

    stops = set()
    # The self-check runs BEFORE the pair under test is opened.
    if pin == SELF_PIN:
        ref = args
    rs = window(ref[0], ref[1], SELF_PIN)
    print("SELF_EARLY=%s" % ref[0])
    print("SELF_LATE=%s" % ref[1])
    emit("SELF_", rs, SELF_PIN)
    ok = abs(rs["E"] - SELF_E) <= SELF_TOL
    print("SELF_CHECK=%s (E %.7f vs %.6f, tol %.6f)" % ("PASS" if ok else "FAIL", rs["E"], SELF_E, SELF_TOL))
    if dead(rs):
        stops.add("D")
    if rs["MULTI_PPS"] > 0:
        stops.add("O")
    if not ok:
        stops.add("S")

    r = rs
    if pin != SELF_PIN:
        print("EARLY=%s" % args[0])
        print("LATE=%s" % args[1])
        r = window(args[0], args[1], pin)
        emit("", r, pin)
        if dead(r):
            stops.add("D")
        if r["MULTI_PPS"] > 0:
            stops.add("O")

    stop = next((s for s in ("D", "O", "S") if s in stops), "none")
    print("SHARES_STOP=%s" % stop)
    if pin == SELF_PIN:
        # The self-check alone never yields a frozen value.
        print("E_FROZEN=SELF_CHECK_ONLY")
    elif stop == "none":
        print("E_FROZEN=%.4f" % r["E"])
    else:
        print("E_FROZEN=WITHHELD")


if __name__ == "__main__":
    main(sys.argv)
