# @topgunbuild/server

WebSocket server for TopGun with clustering, PostgreSQL adapter, and real-time sync.

## Security

### Debug Endpoints

Debug endpoints expose internal CRDT state, operation history, search statistics, and conflict resolution details. **These endpoints should NEVER be enabled in production environments.**

#### Available Debug Endpoints

| Endpoint | Method | Exposed Data |
|----------|--------|--------------|
| `/debug/crdt/export` | POST | Complete CRDT operation history (JSON/CSV/NDJSON) |
| `/debug/crdt/stats` | POST | CRDT statistics (operation counts, conflict rates) |
| `/debug/crdt/conflicts` | POST | Resolved conflicts with timestamps and values |
| `/debug/crdt/operations` | POST | Queryable operation log (by map, node, type) |
| `/debug/crdt/timeline` | POST | Time-series data of CRDT operations |
| `/debug/search/explain` | POST | Search query execution plans and debug info |
| `/debug/search/stats` | GET | Search performance statistics |
| `/debug/search/history` | POST | Historical search queries and results |

#### Security Implications

These endpoints:

- **Expose internal state** - Complete operation history reveals all data changes
- **Leak sensitive information** - Conflict resolution shows concurrent writes and their values
- **Enable timing attacks** - Statistics reveal usage patterns
- **No authentication** - Endpoints are unprotected when enabled

#### Configuration

Debug endpoints are controlled by the `TOPGUN_DEBUG_ENDPOINTS` environment variable:

```bash
# Disable debug endpoints (RECOMMENDED for production)
TOPGUN_DEBUG_ENDPOINTS=false

# Enable debug endpoints (ONLY for development/debugging)
TOPGUN_DEBUG_ENDPOINTS=true
```

**Default:** `false`

When enabled, the server emits a warning log at startup listing all exposed endpoints.

#### Note on TOPGUN_DEBUG

The `TOPGUN_DEBUG` environment variable controls general debug logging and does NOT enable debug endpoints. The two variables are intentionally separate to prevent accidental exposure in production.

### Health Endpoints

The following endpoints are always enabled and safe for production:

- `GET /health` - Returns `{"status": "ok", "timestamp": "..."}`
- `GET /ready` - Returns `{"ready": true}`

These endpoints do not expose sensitive information.

## License

MIT
