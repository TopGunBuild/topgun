# spec376-fixtures — hand-built `/proc` samples for the Linux memory sampler

Inputs for the synthetic cases M1–M5 of `spec376-synth.sh`, read by `procmem_row <pid> <proc_root>` of
`spec376-procmem.sh` in fixture mode. Field names, order and padding follow Debian 12 (kernel 6.1):
`smaps_rollup` is the `[rollup]` header line followed by `%-16s%8llu kB` per field (`Pss_Dirty` present since 6.0);
`status` is `Key:\t%8lu kB` for the `Vm*`/`Rss*` lines. Every number is KiB. The hand-typed values satisfy the
kernel's own identities (`Rss = Shared_Clean + Shared_Dirty + Private_Clean + Private_Dirty`,
`VmRSS = RssAnon + RssFile + RssShmem`, `Pss = Pss_Anon + Pss_File + Pss_Shmem`), so a case tests the sampler and
not an impossible input — except M5, which breaks R0.2 on purpose.

## Layout

Each case directory is a proc root for pid `4242`: `<case>/4242/{smaps_rollup,status}`, `<case>/4242.alive`,
`<case>/4242.ps_rss`. The fixture-mode contract (what selects fixture mode, what `.alive` and `.ps_rss` mean, and
that a proc-root override is permitted only for these fixtures) is normative in `spec376-manifest.md` §1,
"Fixtures"; it is not restated here so the two cannot drift.

## Cases and hand-computed reference values

Values are the R0.1 memory columns in header order, `%.3f` of integer KiB divided once by 1024.
`fp_equiv_mb = (Anonymous − LazyFree + Swap) / 1024`, `file_mb = (Private_Clean + Shared_Clean) / 1024`.

| case | dir | alive | reference |
|---|---|---|---|
| M1 | `m1-alive-nolazy` | 1 | `rss_mb=597.992 fp_equiv_mb=576.051 hwm_rss_mb=640.000 lazyfree_mb=0.000 swap_mb=0.000 file_mb=21.902 anon_mb=576.051 private_dirty_mb=576.090 pss_mb=593.867 anon_huge_mb=336.000 smaps_rss_mb=597.992`; invariants hold; `fp_equiv_mb = anon_mb` |
| M2 | `m2-lazy-swap` | 1 | `rss_mb=500.000 fp_equiv_mb=416.750 hwm_rss_mb=584.000 lazyfree_mb=64.000 swap_mb=12.000 file_mb=95.250 anon_mb=468.750 private_dirty_mb=404.750 pss_mb=492.188 anon_huge_mb=0.000 smaps_rss_mb=500.000`; `fp_equiv_mb = (480000 − 65536 + 12288) / 1024`; invariants hold |
| M3 | `m3-missing-lazyfree` | 1 | M1's files with the `LazyFree:` line removed ⇒ a required field is missing on a live pid ⇒ the helper's fatal status |
| M4 | `m4-gone` | 0 | no `4242/` dir and no `4242.ps_rss` ⇒ empty memory cells, post-mortem memory-row counter = 1 |
| M5 | `m5-lazy-gt-anon` | 1 | `LazyFree 120000 > Anonymous 100000` ⇒ R0.2 violated; the row is still written (`fp_equiv_mb=-19.531`), invariant counter = 1 |

After the smoke, the real sample captured from the `sc` server (`spec376-smaps-sample.txt`) is copied here and
committed at M.
