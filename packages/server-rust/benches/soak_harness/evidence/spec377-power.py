#!/usr/bin/env python3
"""Predicted readings and power for the Linux allocator series (manifest section 1).

Runs the decision gate's own estimator -- the census join (nearest live row
within 30 s), live(t) interpolated between retained census points, pe, the
level over the last LEVEL_WINDOW_S, OLS over rows [floor(n/2) .. n-1] (the
frozen fitter's last-half window), the AR(1)-adjusted relative se e and the
two-sided classes -- over the only committed long series: three 4-hour M1
cells (phys_footprint_mb, 60 s rows, census every 300 s). It is a prediction,
never a gate: no number here is a threshold for the Linux series.

The M1 A0 (16.64 MiB) is used for these M1 cells only; the Linux programs read
A0_L_MIB from section 1. B, STAGE2_MAX_H and LEVEL_WINDOW_S are read from
section 1 of spec377-manifest.md, never retyped here.

Output:
  TABLE <row>   the section-1 table rows, byte for byte as the manifest prints
                them (the AC transcript diffs them against the manifest);
  SPREAD <text> the Linux per-row spread fragment the manifest quotes;
  the rest      diagnostics (4 h gate reading, the recorded fixed-memory fit).
Run: LC_ALL=C python3 spec377-power.py
"""
import csv
import math
import os
import re
import sys

EV = os.path.dirname(os.path.abspath(__file__))
MANIFEST = os.path.join(EV, "spec377-manifest.md")
A0_M1 = 16.64          # MiB, the M1 cells' own startup level (SPEC-372's frozen constant)
D_M1 = 14400.0         # the M1 cells are 4 h
JOIN_S = 30.0
W6 = 3.0               # the last-half window of a 6 h cell, hours


def section1_literal(key):
    """The first token of the one column-0 KEY= line above the APPEND-ONLY marker."""
    lines = []
    with open(MANIFEST, encoding="utf-8") as fh:
        for line in fh:
            if line.startswith("## APPEND-ONLY BELOW"):
                break
            lines.append(line.rstrip("\n"))
    hits = [l for l in lines if l.startswith(key + "=")]
    if len(hits) != 1:
        sys.exit(f"FATAL: section 1 must carry {key}= exactly once (found {len(hits)})")
    return float(hits[0][len(key) + 1:].split()[0])


B = section1_literal("FLAT_BAR_PER_H") * 100.0        # %/h
STAGE2_MAX_H = section1_literal("STAGE2_MAX_H")
LEVEL_WINDOW_S = section1_literal("LEVEL_WINDOW_S")


def census_and_rows(base):
    rows = [(float(r["elapsed_secs"]), float(r["phys_footprint_mb"]))
            for r in csv.DictReader(open(os.path.join(EV, base + ".csv"))) if r["phys_footprint_mb"]]
    live_rows = [r for r in rows if r[0] < D_M1]
    cen = []
    for line in open(os.path.join(EV, base + ".harness-console.log"), errors="ignore"):
        m = re.search(r"(LIVE_COPY|TERMINAL)\s+t=([\d.]+)s.* live=(\d+)", line)
        if m:
            cen.append((float(m.group(2)), int(m.group(3))))
    kept = sorted(c for c in cen if min(abs(r[0] - c[0]) for r in live_rows) <= JOIN_S)
    return live_rows, kept


def pe_series(base):
    live_rows, kept = census_and_rows(base)
    pe = []
    for t, f in live_rows:
        for (ta, la), (tb, lb) in zip(kept, kept[1:]):
            if ta <= t <= tb:
                live = la + (lb - la) * (t - ta) / (tb - ta)
                pe.append((t / 3600.0, (f - A0_M1) * 1048576.0 / live))
                break
    return pe


def classify(pe, w_start_h):
    """w_start_h None = the gate window, rows [floor(n/2) .. n-1]; else rows at t >= w_start_h."""
    lvl = [p[1] for p in pe if p[0] * 3600 >= D_M1 - LEVEL_WINDOW_S]
    level = sum(lvl) / len(lvl)
    w = pe[len(pe) // 2:] if w_start_h is None else [p for p in pe if p[0] >= w_start_h]
    xs = [p[0] for p in w]
    ys = [p[1] for p in w]
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    sxx = sum((x - mx) ** 2 for x in xs)
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sxx
    a = my - b * mx
    res = [y - (a + b * x) for x, y in zip(xs, ys)]
    se = math.sqrt(sum(r * r for r in res) / (n - 2) / sxx)
    m = sum(res) / n
    v = sum((r - m) ** 2 for r in res)
    r1 = sum((res[i] - m) * (res[i + 1] - m) for i in range(n - 1)) / v
    k = 1.0 if r1 < 0 else (math.sqrt((1 + r1) / (1 - r1)) if r1 < 0.99 else float("inf"))
    return n, level, b / level * 100, r1, se / level * k * 100


def verdict(s, e):
    if s - 2 * e > B:
        return "RISING"
    if s + 2 * e < -B:
        return "FALLING"
    if abs(s) + 2 * e <= B:
        return "FLAT"
    if 2 * e > B:
        return "UNDERPOWERED"
    return "MARGINAL"


def signed(x):
    """The manifest's signed form: '+1.34', and a true minus sign for negatives."""
    s = f"{x:+.2f}"
    return "−" + s[1:] if s.startswith("-") else s


def grouped(x):
    """An integer with a space every three digits ('2 020')."""
    return f"{x:,.0f}".replace(",", " ")


CELLS = [("spec372-j2", "`j2` (JE)", "arm"), ("spec372-m2", "`m2` (MI v3)", "arm"), ("spec372-s2", "`s2` (SYS)", "sys")]

print("# diagnostics: 4 h gate reading (W=2h) | W=3h empirical | 6 h projected e6 = e4 x (2/3)^1.5")
for base, label, kind in CELLS:
    pe = pe_series(base)
    n, level, s, r1, e = classify(pe, None)
    n3, level3, s3, r13, e3 = classify(pe, 1.0)
    e6 = e * (2.0 / 3.0) ** 1.5
    cls = verdict(s, e6)
    flat_room = B - 2 * e6
    print(f"{base} n={n} PE_LEVEL={level:.0f}B s={s:+.2f}%/h r1={r1:.2f} e4={e:.2f} class4={verdict(s, e)} "
          f"e3={e3:.2f} class3={verdict(s3, e3)} e6={e6:.2f} 2e6={2 * e6:.2f} class6={cls}")
    if kind == "arm":
        predicted = f"`{cls}` ⇒ `UNRESOLVED({cls})`" if cls in ("FALLING", "UNDERPOWERED", "MARGINAL") else f"`{cls}`"
    else:
        t2 = max(6, math.ceil(2 * W6 * (2 * e6 / (0.8 * B)) ** (2.0 / 3.0)))
        predicted = f"`{cls}`; `STAGE2_T` = {t2} h ⇒ " + ("infeasible" if t2 > STAGE2_MAX_H else "feasible")
    cmp_ = "<" if 2 * e6 < B else ">"
    room = f"{flat_room:.2f}" if flat_room > 0 else "none"
    print(f"TABLE | {label} | {grouped(level)} B | {signed(s)} | {r1:.2f} | {e:.2f} | {e3:.2f} | **{e6:.2f}** | "
          f"{2 * e6:.2f} {cmp_} {B:g} | {room} | {predicted} |")

# Recorded only (never a decision input): the fixed-memory fit over the gate
# window. It is not identifiable while live grows near-linearly.
print("# recorded: fixed-memory fit fp = F0 + MC x live over the gate window (M1 cells)")
for base, label, kind in CELLS:
    live_rows, kept = census_and_rows(base)
    pts = []
    for t, f in live_rows:
        for (ta, la), (tb, lb) in zip(kept, kept[1:]):
            if ta <= t <= tb:
                pts.append((t / 3600.0, la + (lb - la) * (t - ta) / (tb - ta), f * 1048576.0))
                break
    pts = pts[len(pts) // 2:]
    ls = [p[1] for p in pts]
    fs = [p[2] for p in pts]
    n = len(pts)
    ml = sum(ls) / n
    mf = sum(fs) / n
    mc = sum((l - ml) * (f - mf) for l, f in zip(ls, fs)) / sum((l - ml) ** 2 for l in ls)
    f0 = (mf - mc * ml) - A0_M1 * 1048576.0
    print(f"{base} FIXED_EST={f0 / 1048576:+.0f}MiB MC={mc:.0f}B/entry")

# Linux per-row spread (descriptive): SPEC-376 900 s CA cells, fp_equiv_mb,
# live rows (elapsed < 900) with elapsed >= 450 s, residual sd around an OLS
# line relative to the last live row.
parts = []
for base, tag in [("spec376-c1", "c1"), ("spec376-c2", "c2"), ("spec376-pa", "pa"), ("spec376-pb", "pb")]:
    rr = [(float(r["elapsed_secs"]) / 3600, float(r["fp_equiv_mb"]))
          for r in csv.DictReader(open(os.path.join(EV, base + ".csv")))
          if r["fp_equiv_mb"] and float(r["elapsed_secs"]) < 900]
    h = [r for r in rr if r[0] >= 0.125]
    xs = [r[0] for r in h]
    ys = [r[1] for r in h]
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
    a = my - b * mx
    sd = math.sqrt(sum((y - (a + b * x)) ** 2 for x, y in zip(xs, ys)) / (n - 2))
    print(f"{base} rows={n} resid_sd_rel={sd / ys[-1] * 100:.2f}%")
    parts.append(f"`{tag}` {sd / ys[-1] * 100:.2f} %")
print("SPREAD " + ", ".join(parts))
