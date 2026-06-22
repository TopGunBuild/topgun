# G4b 72h soak — operator runbook

The full 72-hour soak is the **final G4b gate**, run on a stabilized build by an
operator on a Hetzner **scratch** box — not in CI, and not on the live demo VPS.
This runbook covers provisioning, launching, monitoring remotely, and reading
the result. The harness itself and its short smoke run are validated separately
(see [`README.md`](./README.md)); this is purely the long endurance run.

## 0. Prerequisites

- A **dedicated scratch** Hetzner instance (Linux). Do **not** use the box that
  serves `demo.topgun.build` — a crash-looping soak would disrupt it. Access and
  provisioning helpers live in `~/Projects/hetzner` (`access.txt`, `dokploy`).
- Rust toolchain on the box (`rustup`), `jq`, and `git`.
- Disk: the soak writes a redb + WAL under the run dir; a few GB of free space is
  ample for the default keyspace.

## 1. Provision a scratch box

Use the helpers in `~/Projects/hetzner` (or the Hetzner console / `dokploy`) to
spin up a fresh instance. Confirm it is **not** the demo box:

```bash
ssh root@<scratch-box> 'hostname -f; curl -fsS --max-time 2 http://127.0.0.1:8080/ || echo "no server on 8080 (good)"'
```

## 2. Get the code onto the box

```bash
ssh root@<scratch-box>
git clone https://github.com/TopGunBuild/topgun.git && cd topgun
# or rsync your working tree if testing an unmerged branch
```

## 3. Launch the soak (detached)

The runner builds release binaries, runs the two negative controls as a
pre-flight (aborting if the harness cannot fail), then launches the soak under
`nohup` so it survives your SSH session.

```bash
cd topgun
# 72h defaults; override any knob via env (see the script header).
DURATION=259200 CRASH_INTERVAL=300 \
  packages/server-rust/benches/soak_harness/hetzner-soak-runner.sh
```

It prints the run directory (`/var/soak/run-<UTC>/`), the PID, and the monitor
commands. The PID is also saved to `<run-dir>/soak.pid`.

### systemd alternative (survives reboots, auto-restart of the *driver*)

If you want the soak driver itself supervised (e.g. so a box reboot resumes it),
wrap the same command in a unit. Note this supervises the harness process, which
in turn supervises the server child:

```ini
# /etc/systemd/system/topgun-soak.service
[Unit]
Description=TopGun G4b 72h soak
After=network.target

[Service]
Type=simple
WorkingDirectory=/root/topgun
Environment=DURATION=259200 CRASH_INTERVAL=300
ExecStart=/root/topgun/packages/server-rust/benches/soak_harness/hetzner-soak-runner.sh
Restart=no

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload && systemctl start topgun-soak
journalctl -u topgun-soak -f
```

## 4. Monitor remotely while it runs

The harness appends one JSON line per checkpoint to `progress.jsonl`, so you can
watch a 72h run in real time from your laptop without attaching:

```bash
# Live progress stream (one line per steady/recovery checkpoint):
ssh root@<scratch-box> 'tail -f /var/soak/run-*/progress.jsonl'

# Human-readable summary of the latest checkpoint:
ssh root@<scratch-box> 'tail -1 /var/soak/run-*/progress.jsonl | jq "{elapsedSecs, phase, totalWrites, crashes, lastConvergenceOk, peakRssMb, panicsSeen}"'

# Watch RSS trend (memory plateau check):
ssh root@<scratch-box> 'jq -r "[.elapsedSecs, .lastRssMb] | @tsv" /var/soak/run-*/progress.jsonl' | tail -50
```

Each progress line carries: `elapsedSecs`, `phase` (`steady`/`recovery`),
`totalWrites`, `writeErrors`, `reconnects`, `crashes`, checkpoint counts,
`lastConvergenceOk`, `peakRssMb`, `lastRssMb`, `panicsSeen`.

## 5. Read the result

When the run ends, the final report is written to `<run-dir>/report.json`:

```bash
ssh root@<scratch-box> 'cat /var/soak/run-*/report.json | jq "{passed, finishedReason, totalWrites, crashes, recoveryCheckpoints, memory, convergenceFailures, recoveryFailures, panicReport}"'

# One-line verdict + exit-style check:
ssh root@<scratch-box> 'jq -e .passed /var/soak/run-*/report.json >/dev/null && echo "G4b SOAK PASS" || echo "G4b SOAK FAIL"'
```

Interpretation:

- `passed: true`, `finishedReason: "duration reached"` → **G4b green**: 72h of
  churn + crash loops with zero divergence, zero recovery mismatch, bounded
  memory (`memory.passed`), zero panics.
- `passed: false` → inspect `convergenceFailures` / `recoveryFailures` /
  `memory.reason` / `panicReport`. Each carries the offending keys, Merkle
  roots, RSS slope, or panic context. A failure here is a **real finding** —
  capture the `run-dir` (it holds the log, progress stream, and the on-disk
  redb + WAL for forensics) and file it.

## 6. Known caveat for the current build (TODO-530)

On the current build the crash-loop path is **RED by design** because of
TODO-530 (after `kill -9` + restart the server serves an empty map although the
data is durably on disk — query/Merkle read only the in-memory layer, which is
not rehydrated on restart). Run the full crash-loop 72h soak only **after**
TODO-530 lands. To exercise everything *except* recovery in the meantime, set
`CRASH_INTERVAL=0` (churn + steady convergence + memory + panic watch).
