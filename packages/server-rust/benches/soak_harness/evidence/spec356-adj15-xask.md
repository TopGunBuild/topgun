# ADJ-15 adversarial cross-vendor round + regression proof (2026-08-08)

Question put to the vendor (verbatim):

> CONTEXT. Same frozen pre-registered classification protocol as before (append-only adjudications, exact-complement partition). One remaining discriminator: within the "PERSISTENT licensed backlog" branch, Step 3 (SCHEDULING: "the prune is run less and less often as the corpus grows") vs Step 4 (THROUGHPUT: prune scheduled fine but can't keep up). Evidence: the cumulative curve passes_total (monotone counter, y) vs current_epoch (monotone, x) over the last half of a 4h run; split into an early slice and a late slice; each slice fit by a pinned auditable-by-inspection OLS awk script; slopes s_early, s_late in passes-per-epoch.
> 
> TWO DEFECTS FOUND BY AUDIT (pre-data, must be adjudicated now):
> (A) The pinned fitter hardwires x = elapsed time. Regressing passes_total against TIME instead of EPOCH gives the OPPOSITE verdict on accelerating-epoch data (probe: time-axis says s_early=s_late → Step 4; epoch-axis says s_early=21600, s_late=7200 → Step 3). Fix chosen (mechanical, not the question): additive backward-compatible -v xcol= parameter, default unchanged, with a regression-proof that all committed historical invocations produce byte-identical output.
> (B) The fitter's own doc-contract says: for cumulative autocorrelated series the OLS standard error is OPTIMISTIC ("understates the true uncertainty, often by a large factor... SE separation alone must never carry a discrimination claim; the minimum effect-size floor does"). The current adjudication made SE-separation-alone the sole Step-3/4 discriminator with no floor — violating the instrument's own contract and biasing toward SCHEDULING (optimistic SE makes "significantly below" too easy).
> 
> THE QUESTION: what should carry the Step 3 claim? My proposal to attack:
> 
> Step 3 (SCHEDULING) requires BOTH:
>   (leg 1, necessary-not-sufficient) s_late significantly below s_early at one-sided alpha=0.05 using the fitter's (admittedly optimistic) SE — kept only as a cheap sanity precondition;
>   (leg 2, the floor that carries the claim) s_late <= 0.5 * s_early — a pre-registered coarse halving floor: "run less and less often" must be a MATERIAL decline, not noise.
> Otherwise Step 4.
> Sensitivity convention (reusing an existing protocol label, no new routing): the verdict is computed at 0.5; it is ALSO computed at 0.4 and 0.6, and if it flips within that band the verdict carries a pre-existing CONTESTED label (which changes reporting emphasis, not routing).
> 
> Project priors you should respect: this project has repeatedly KILLED derived-formula bounds as false precision (a slope-noise formula and a churn formula both died; a fixed coarse margin won each time); it also killed a guard-band proposal that would have destroyed an exact-complement partition (the floor here does NOT create a third branch — Step 4 is simply "not Step 3").
> 
> QUESTIONS:
> 1. Attack the two-leg structure. Is keeping the optimistic-SE leg as a precondition useful, harmful, or theater? Would floor-only be more honest?
> 2. Attack the halving floor: construct regimes where 0.5 misclassifies (e.g., s_early inflated by an early burst; genuine scheduling decay that is slow but compounding — 0.8x per half repeated; near-zero s_early degeneracy). Is a relative floor the right SHAPE at all, or does the claim need an absolute leg (minimum pass rate) to avoid degeneracy at tiny s_early?
> 3. Is the 0.4/0.6 CONTESTED sensitivity convention sound, or does it smuggle a band back in?
> 4. The asymmetry check: does this construction bias toward either verdict through instrument limitations alone (the protocol's cardinal sin)? Consider: epoch axis vs time axis, autocorrelation, slice boundary placement, early-burst contamination.
> Concise, findings-first.

Vendor answer (verbatim, z-ai/glm-5.2 via openrouter, cost $0.0036):

> → openrouter z-ai/glm-5.2 answering (0 file(s) embedded)…
> 
> [usage] prompt=961 completion=4216 total=5177 cost=$0.0036012904
> ## Findings
> 
> ### 1. The SE precondition is theater with a back door
> 
> **Verdict: drop it. Floor-only is more honest.**
> 
> The SE leg can only be binding in one scenario: floor met (s_late ≤ 0.5·s_early) but optimistic-SE says "not significant." In that case the precondition routes to Step 4 — overriding the material-decline floor using the very instrument whose contract says it *cannot* carry the claim. You've built a gate from a tool you've discredited. The two cases where the SE passes are redundant (floor already carried it); the one case where it fails is the one where it shouldn't be allowed to speak.
> 
> If you want a sanity gate against garbage fits, gate on fit quality (residual structure, R²) or on minimum slice length — not on the slope-difference SE of an autocorrelated cumulative series. The floor is your discriminator. Let it discriminate.
> 
> ### 2. The halving floor misclassifies in at least three regimes; the shape is right but needs an absolute guard
> 
> **Regimes where 0.5 relative fails:**
> 
> | Regime | Mechanism | Misclassification direction |
> |---|---|---|
> | **Early-burst inflation** | Warmup/initialization inflates s_early; true steady-state ratio is 0.7 but burst makes measured ratio 0.4 | False Step 3 (bias toward scheduling) |
> | **Slow compounding decay** | Genuine scheduling decay at 0.8× per half; real Step 3 but never crosses 0.5 | False Step 4 (misses real scheduling) |
> | **Near-zero degeneracy** | s_early = 8, s_late = 3; floor met but both are noise-floor throughput | False Step 3 (amplifies denominator noise) |
> | **Late stall** | s_late → 0 from either cause; floor trivially met | Conflates scheduling stall with throughput stall |
> 
> **Shape assessment:** Relative is the right semantic for "less and less often" — it captures the *rate of decay*, which is what scheduling frequency change means. But a relative-only floor is degenerate at small s_early. You need a **coarse absolute floor on s_early** (minimum passes-per-epoch for the measurement to be informative). This is not a derived formula — it's a fixed coarse margin, consistent with project priors. Framed as "if s_early is below this, the instrument can't discriminate → Step 4," it preserves the exact-complement partition.
> 
> The slow-compounding-decay regime (0.8× per half) is the harder problem. A 0.5 floor genuinely can't catch it. But that's a sensitivity limitation, not a bias — and the CONTESTED band partially addresses it (a 0.6 verdict would fire). The alternative (lowering the floor to 0.8) would make early-burst false positives much worse. 0.5 is a reasonable operating point if you accept that slow decay is below detection threshold.
> 
> ### 3. CONTESTED is sound *if* the no-routing constraint is enforced — but it's one convention violation away from a guard band
> 
> The killed guard-band proposal created a third routing destination. CONTESTED does not — it's a reporting flag on a binary verdict. The partition survives. This is a real distinction, not cosmetic.
> 
> **The risk:** CONTESTED becomes a de facto third branch the moment any downstream consumer (human or automated) treats it differently — reruns, escalates, applies different weights, or "holds for review." The protocol needs an explicit, enforceable rule: CONTESTED changes exactly zero routing decisions. If you can't guarantee that in practice, you've smuggled the guard band back in through a side door.
> 
> **The band-width question:** 0.4/0.6 is itself a free parameter. Pre-registration mitigates post-hoc tuning, but the choice still needs justification beyond "seemed reasonable." If it's arbitrary, it's a degree of freedom you're not accounting for.
> 
> ### 4. Instrument-driven bias: mild but systematic, toward Step 3
> 
> | Source | Bias direction | Severity |
> |---|---|---|
> | **Early-burst contamination of s_early** | Toward Step 3 (inflated denominator → floor easier to meet) | **High** — warmup is ubiquitous, and the relative floor amplifies it |
> | **Optimistic SE precondition** (if retained) | Toward Step 3 (too easy to pass) | Moderate — only matters when SE is binding, which it shouldn't be if floor is the discriminator |
> | **Epoch vs time axis** | Toward Step 4 if time axis is default (per defect A probe) | **High but sidestepped** — the fix preserves default behavior, so if the pre-registered protocol used the default, the pre-registration itself used the wrong axis. "Mechanical, not the question" doesn't eliminate the bias; it defers it. |
> | **Slice boundary placement** | Direction depends on placement; if boundary is in the transition zone, both slices contaminated | Moderate — the relative floor is more sensitive to this than an absolute measure |
> | **Autocorrelation in cumulative series** | Doesn't bias the *point estimate* of slopes, only the SE — so floor-only is immune | Eliminated if you drop the SE leg |
> 
> **The cardinal sin check:** The construction does introduce instrument-driven asymmetry, primarily through early-burst inflation of s_early interacting with the relative floor. This is a structural bias, not a random one — it systematically favors Step 3 when warmup artifacts exist. The absolute-floor guard on s_early doesn't fix this (the burst inflates s_early *above* the absolute floor, which is the problem). The real fix is slice-boundary robustness: either use a later start for the "early" slice (past warmup) or report sensitivity to boundary placement. If you can't do either, acknowledge that warmup contamination biases toward Step 3 and the protocol is conservative against Step 4 only in the no-warmup regime.
> 
> **Bottom line:** Floor-only with an absolute guard on s_early, drop the SE leg, keep CONTESTED with an iron-clad no-routing rule, and flag early-burst contamination as the residual uncontrolled bias. The epoch-axis issue is not actually "not the question" — if the pre-registered protocol defaulted to time axis, the pre-registration is defective and the fix doesn't repair it for going-forward adjudications.

## Conductor divergence from the vendor, recorded

The vendor routed the small-s_early degeneracy to Step 4 to preserve the partition.
ADJ-15 routes it to Step 0(c) INDETERMINATE instead: the protocol already owns a
sanctioned non-branch exit (ADJ-12 set the precedent), and defaulting a regime the
instrument cannot read to EITHER branch is a lean. Everything else is adopted as
answered: SE leg dropped, absolute guard added, CONTESTED iron rule verbatim.

## Regression proof: the xaxis extension changes no committed reading

Old fitter = `git show f2f72c62:.../spec349c2-fit.awk`; new = HEAD. Every committed
evidence CSV x {full,last_half} x {rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes}:

    lines compared: 190
    cmp: byte-identical (exit 0)

New-mode probe (the fresh armed smoke prune.csv, passes_total vs current_epoch):

    col=passes_total window=full rows_used=12 n=12 skipped_empty=0 xaxis=current_epoch x_start=2.0 x_end=6.0 x_span=4.0 y_first=365.000 y_last=4495.000 slope_per_x_unit=950.394422 se_per_x_unit=65.258090 intercept=-1276.294821 r2=0.954975 sxx=20.916666667 sse=890760.996015936
