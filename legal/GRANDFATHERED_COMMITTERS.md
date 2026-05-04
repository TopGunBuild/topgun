# Grandfathered Committers

This file snapshots all authors with merged commits in `TopGunBuild/topgun` as of the date below. Every commit by these authors landed under the repo's existing Apache License 2.0 (see `LICENSE`), which grants TopGun the right to use, distribute, and sublicense their contribution under Apache-2.0 terms.

For any future relicensing of specific files to BSL (per the open-core licensing strategy), the maintainer must verify that no grandfathered committer's code is materially present in those specific files, or obtain retroactive sign-off. This list is the audit baseline.

## Snapshot

**Date:** 2026-05-04
**Reproducibility command:** `git log --no-use-mailmap --format='%aN <%aE>' | sort -u`
**Branch:** `v2.0-data-platform` (used at execution time; `main` is 395+ commits behind per SPEC-232 finding — `v2.0-data-platform` is the more-current branch and its author set is a strict superset of `main`'s author set)

```
Ivan Kalashnik <easysolpro@gmail.com>
kiborg <easysolpro@gmail.com>
semantic-release-bot <semantic-release-bot@martynus.net>
```

## Notes

- This is a snapshot, not a live list. Re-running the command at a later date will include post-snapshot contributors; those contributors are bound by the CLA they signed via cla-assistant.io and do NOT need to be added here.
- The CLA grant supersedes the implicit Apache-2.0 grant for post-snapshot contributors — they have explicitly granted relicensing rights, so BSL relicensing of their contributions is permitted without further sign-off.
- The `--no-use-mailmap` flag ensures snapshot reproducibility even if a `.mailmap` file is added or modified after the snapshot date — author names recorded here are the raw commit-author names, not the mailmap-rewritten display names.
