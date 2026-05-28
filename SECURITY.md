# Security Policy

Thanks for taking the time to responsibly disclose vulnerabilities in TopGun. We take security seriously and will work with you to resolve issues quickly.

## Reporting a vulnerability

**Please do not open public GitHub issues for security bugs.**

Use one of these private channels instead:

- **Preferred:** [GitHub Security Advisories](https://github.com/TopGunBuild/topgun/security/advisories/new) — encrypted, threaded, integrates with our patch workflow.
- **Alternative:** email `security@topgun.build` with the details below.

### What to include

- A description of the issue and its impact (data exposure, RCE, auth bypass, etc.).
- The affected component (`@topgunbuild/client`, `topgun-server`, `mcp-server`, …) and version.
- Reproduction steps, proof-of-concept, or a failing test if you have one.
- Whether you intend to publish a public write-up, and the disclosure timeline you'd like to follow.

### What to expect

| Step | Target |
|---|---|
| Acknowledgement of your report | within **48 hours** |
| Initial triage + severity assignment | within **5 business days** |
| Coordinated fix + advisory drafted | within **30 days** for high/critical, **90 days** for medium/low |
| Public advisory + patched release | by mutual agreement on the disclosure date |

We will credit you in the advisory unless you ask to remain anonymous.

## Supported versions

| Version | Status |
|---|---|
| `2.x` | Active — security fixes shipped as patch releases. |
| `1.x` (legacy `gun.js` port) | End-of-life. No security fixes. |
| `0.x` (pre-release internal milestones) | End-of-life. Upgrade to `2.x`. |

## Scope

In scope:

- Server: `packages/server-rust/` (axum WS handler, clustering, redb/Postgres backends, eviction, write-behind, auth, ACL).
- Clients: `packages/client/`, `packages/react/`, `packages/adapters/`.
- Tooling: `packages/mcp-server/`, `packages/create-topgun-app/`, `packages/cli/`.
- Deployment: `deploy/Dockerfile.*`, `deploy/k8s/`, root `docker-compose.yml`.
- The `topgun.build` documentation site (apps/docs-astro).

Out of scope:

- Third-party services we integrate with (Clerk, BetterAuth, Firebase) — please report those upstream.
- Example apps (`examples/notes-app`, `examples/sync-lab`) — these are demos. Issues affecting only the demos can be filed as regular GitHub issues.
- Self-signed test certificates committed historically to test fixtures (see `legal/HISTORICAL_DISCLOSURES.md` if/when published) — they were never trusted by any third party.

## Safe-harbor

We will not pursue legal action against researchers who:

- Make a good-faith effort to follow this policy.
- Avoid privacy violations, data destruction, and service degradation while testing.
- Give us a reasonable window to fix the issue before public disclosure.

If you are unsure whether your activity falls under safe-harbor, please ask before testing.

## Cryptography and dependencies

The Rust server links OpenSSL, rustls, jsonwebtoken, and similar crypto-bearing crates. We follow upstream CVE feeds via `cargo audit`. If you spot a transitive vulnerability in our lockfile, the fastest path is a security advisory naming the crate and version range.
