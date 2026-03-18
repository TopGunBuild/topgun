# Production Tuning Guide

Recommendations for deploying TopGun's Rust server under high-connection workloads on Linux.

## File Descriptor Limits

Each WebSocket connection consumes one file descriptor. The default `ulimit -n` on most Linux distributions is **1024**, which limits the server to roughly 1000 concurrent connections after accounting for internal fds (log files, database connections, epoll).

**For high connection counts:**

```bash
# Temporary (current shell session)
ulimit -n 1048576

# Persistent via /etc/security/limits.conf
*    soft    nofile    1048576
*    hard    nofile    1048576
```

**For systemd-managed services**, set in the unit file:

```ini
[Service]
LimitNOFILE=1048576
```

Also verify the system-wide limit:

```bash
# Check current system max
cat /proc/sys/fs/file-max

# Increase if needed (e.g., for 500k+ connections)
sysctl -w fs.file-max=2097152
```

## TCP Buffer Tuning

Default kernel TCP buffers allocate approximately **45 KB per connection** (send buffer + receive buffer + overhead). At 100k connections, that's ~4.5 GB of kernel memory for TCP buffers alone.

**Reduce per-connection buffer sizes** for high connection counts where messages are small (TopGun's MsgPack frames are typically < 1 KB):

```bash
# System-wide defaults (bytes)
sysctl -w net.core.rmem_default=8192
sysctl -w net.core.wmem_default=8192

# TCP auto-tuning range: min, default, max (bytes)
sysctl -w net.ipv4.tcp_rmem="4096 8192 16384"
sysctl -w net.ipv4.tcp_wmem="4096 8192 16384"
```

Application-level socket options (set before listen):

| Option | Recommended | Effect |
|--------|-------------|--------|
| `SO_SNDBUF` | 8192 | 8 KB send buffer per socket |
| `SO_RCVBUF` | 8192 | 8 KB receive buffer per socket |

**Trade-off:** Smaller buffers reduce memory usage but increase the number of syscalls for large messages, since the kernel must wake the application more frequently. For TopGun's typical small-frame workload, 8 KB buffers are sufficient.

## Memory Budget

| TCP Buffer Setting | Per-Connection Memory | Connections per 16 GB RAM |
|--------------------|-----------------------|---------------------------|
| Default (~45 KB) | ~45 KB | ~250,000 |
| Reduced (8 KB send + 8 KB recv) | ~17 KB | ~900,000 |

These estimates cover TCP buffer memory only. Application-level memory is additive:

- **CRDT state:** Each map entry (key + LWW record + HLC timestamp) consumes memory proportional to data size
- **Query registry:** Active subscriptions consume memory per registered query
- **Partition stores:** 271 partitions × number of maps × record count
- **Search indexes:** Tantivy indexes consume memory proportional to indexed documents

**Recommendation:** Monitor RSS via `/proc/[pid]/status` and set alerting thresholds at 70% and 90% of available memory.

## Ephemeral Port Exhaustion

The Linux ephemeral port range is typically **32768–60999** (~28,000 ports). Each connection from a single source IP consumes one ephemeral port from the client's perspective.

**Server-side:** Inbound connections share the server's listening port, so ephemeral port exhaustion is generally not an issue for the server itself. It becomes relevant for:

- Load test clients opening many connections from one machine
- Reverse proxies or load balancers connecting to the server

**Mitigation for load testing or proxy scenarios:**

```bash
# Expand ephemeral port range
sysctl -w net.ipv4.ip_local_port_range="1024 65535"

# Enable port reuse for TIME_WAIT sockets
sysctl -w net.ipv4.tcp_tw_reuse=1
```

For >28k connections from a single client IP, use multiple source IPs or `SO_REUSEPORT`.

**`SO_REUSEPORT`** also enables multiple server processes to share the same listening port, allowing kernel-level load balancing across processes:

```bash
# Multiple TopGun instances on the same port
sysctl -w net.core.somaxconn=65535
```

## Tokio Runtime Tuning

TopGun's server runs on the tokio async runtime. By default, tokio spawns one worker thread per CPU core, which is sufficient for most workloads.

**Worker threads:**

```bash
# Override worker thread count via environment variable
TOKIO_WORKER_THREADS=16 ./topgun-server
```

Or configure programmatically via `tokio::runtime::Builder::new_multi_thread().worker_threads(N)`.

**When to adjust:**

| Scenario | Recommendation |
|----------|----------------|
| < 50k concurrent tasks | Default (1 thread per core) is fine |
| 50k–100k concurrent tasks | Monitor task queue depth; adjust if latency increases |
| > 100k concurrent tasks | Explicitly set `worker_threads` to 2× CPU cores |
| Mixed CPU-bound + I/O-bound work | Use `spawn_blocking` for CPU-bound work to avoid starving I/O tasks |

**Backpressure indicator:** If clients receive HTTP 429 responses, the server's partition dispatch channels are full (`PartitionDispatcher` uses `try_send()` with a 256-slot buffer). This indicates worker saturation — either increase worker threads or reduce connection count per instance.

## Connection Monitoring

**OS-level monitoring:**

```bash
# Summary of socket states
ss -s

# Detailed connection counts by state
ss -tn state established | wc -l

# Kernel socket memory usage
cat /proc/net/sockstat
```

**Alerting thresholds** relative to your configured fd limit:

| Level | Threshold | Action |
|-------|-----------|--------|
| Info | 50% of `LimitNOFILE` | Log for capacity planning |
| Warning | 80% of `LimitNOFILE` | Scale up or shed load |
| Critical | 95% of `LimitNOFILE` | Reject new connections, alert on-call |

**Process-level metrics:**

```bash
# File descriptors in use by the TopGun process
ls /proc/$(pgrep topgun-server)/fd | wc -l

# Memory usage
grep -E 'VmRSS|VmSize' /proc/$(pgrep topgun-server)/status
```

TopGun emits structured logs via `tracing` (controlled by `RUST_LOG`). For production monitoring, set:

```bash
RUST_LOG=topgun_server=info
```

This logs connection lifecycle events (open/close) and error conditions without the overhead of debug-level tracing.
