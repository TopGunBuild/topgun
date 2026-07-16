#!/usr/bin/env bash
# gsc.sh — query Google Search Console for topgun.build (the docs/landing site).
#
# Usage:
#   apps/docs-astro/scripts/gsc.sh                      # top queries, last 28d
#   apps/docs-astro/scripts/gsc.sh --dim page           # top pages instead of queries
#   apps/docs-astro/scripts/gsc.sh --dim query,page     # which page ranks for which query
#   apps/docs-astro/scripts/gsc.sh --days 7             # different window
#   apps/docs-astro/scripts/gsc.sh --limit 50           # rows to print (default 25)
#   apps/docs-astro/scripts/gsc.sh --raw                # raw JSON instead of a table
#   apps/docs-astro/scripts/gsc.sh --sites              # properties the service account can see
#   apps/docs-astro/scripts/gsc.sh --site sc-domain:example.com   # another property
#
# CREDENTIALS ARE NOT IN THIS REPO — and must never be. This is a public
# repository; the service account key is a real RSA private key. It lives at
#   ~/.config/gsc/service-account.json   (chmod 600)
# outside every git working tree, so there is nothing here to leak. Override the
# location with GSC_KEY_FILE if yours differs. Access is read-only (GSC role
# "Restricted", scope webmasters.readonly) and the key never appears in output
# or in process args — it is written to a temp PEM that is trapped and removed.
#
# Caveats that bite:
#   - Data lags 2-3 days. --days counts back from (today - 3), not from today.
#   - Anonymized queries (asked by fewer than a few dozen people over 2-3 months)
#     are dropped from query rows entirely, while still counting in GSC's own
#     totals. On a low-traffic site that is most of the long tail: read --dim query
#     as "what already has traction", never as a map of demand. --dim page is the
#     fuller view.
#   - The API's own default rowLimit is 1000 (a silent truncation). We always send
#     it explicitly.
#   - topgun.build is small (as of 2026-07-16: 12 URLs, 2 clicks / 130 impressions
#     per 28d). Track impressions and position; click counts are noise at this size.
set -euo pipefail

KEY_PATH="${GSC_KEY_FILE:-$HOME/.config/gsc/service-account.json}"
GSC_SITE_URL="${GSC_SITE_URL:-sc-domain:topgun.build}"

if [ ! -f "$KEY_PATH" ]; then
  echo "error: service account key not found: $KEY_PATH" >&2
  echo "       It is deliberately kept outside the repo (this repo is public)." >&2
  echo "       Place the GSC service account JSON there (chmod 600), or set" >&2
  echo "       GSC_KEY_FILE to its location." >&2
  exit 1
fi

SITES=0; RAW=0; DAYS=28; DIM="query"; LIMIT=25; TYPE="web"
while [ $# -gt 0 ]; do
  case "$1" in
    --sites) SITES=1; shift ;;
    --raw)   RAW=1; shift ;;
    --days)  DAYS="$2"; shift 2 ;;
    --dim)   DIM="$2"; shift 2 ;;
    --limit) LIMIT="$2"; shift 2 ;;
    --type)  TYPE="$2"; shift 2 ;;
    --site)  GSC_SITE_URL="$2"; shift 2 ;;
    *) echo "error: unknown arg '$1'. See header of $0." >&2; exit 1 ;;
  esac
done

API="https://www.googleapis.com/webmasters/v3"

# --- mint an access token from the service account key -----------------------
PEM="$(mktemp)"; trap 'rm -f "$PEM"' EXIT; chmod 600 "$PEM"
KEY_PATH="$KEY_PATH" python3 -c '
import json, os, sys
with open(os.environ["KEY_PATH"]) as f: k = json.load(f)
sys.stdout.write(k["private_key"])
' > "$PEM"

CLIENT_EMAIL="$(KEY_PATH="$KEY_PATH" python3 -c '
import json, os
with open(os.environ["KEY_PATH"]) as f: print(json.load(f)["client_email"])
')"

b64url() { openssl base64 -A | tr "+/" "-_" | tr -d "="; }

NOW="$(date +%s)"
JWT_HEADER="$(printf '{"alg":"RS256","typ":"JWT"}' | b64url)"
JWT_CLAIM="$(CLIENT_EMAIL="$CLIENT_EMAIL" NOW="$NOW" python3 -c '
import json, os
now = int(os.environ["NOW"])
print(json.dumps({
    "iss": os.environ["CLIENT_EMAIL"],
    "scope": "https://www.googleapis.com/auth/webmasters.readonly",
    "aud": "https://oauth2.googleapis.com/token",
    "iat": now, "exp": now + 3600,
}, separators=(",", ":")))
' | b64url)"

JWT_SIG="$(printf '%s.%s' "$JWT_HEADER" "$JWT_CLAIM" | openssl dgst -sha256 -sign "$PEM" -binary | b64url)"
ASSERTION="$JWT_HEADER.$JWT_CLAIM.$JWT_SIG"

TOKEN_RESP="$(curl -s -m 30 -X POST https://oauth2.googleapis.com/token \
  --data-urlencode "grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer" \
  --data-urlencode "assertion=$ASSERTION")"

ACCESS_TOKEN="$(RESP="$TOKEN_RESP" python3 -c '
import json, os, sys
d = json.loads(os.environ["RESP"])
if "access_token" not in d:
    sys.exit("auth failed: %s — %s" % (d.get("error", "?"), d.get("error_description", "")))
print(d["access_token"])
')"

# --- list properties ---------------------------------------------------------
if [ "$SITES" = 1 ]; then
  curl -s -m 30 "$API/sites" -H "Authorization: Bearer $ACCESS_TOKEN" \
  | python3 -c '
import json, sys
d = json.load(sys.stdin)
entries = d.get("siteEntry", [])
if not entries:
    print("(no properties — is the service account added as a user in GSC?)"); raise SystemExit
for s in entries:
    print("%-40s %s" % (s["siteUrl"], s["permissionLevel"]))
'
  exit 0
fi

# --- search analytics --------------------------------------------------------
END="$(date -v-3d +%F 2>/dev/null || date -d '3 days ago' +%F)"
START="$(date -v-3d -v-"${DAYS}"d +%F 2>/dev/null || date -d "$((DAYS + 3)) days ago" +%F)"

BODY="$(START="$START" END="$END" DIM="$DIM" TYPE="$TYPE" python3 -c '
import json, os
print(json.dumps({
    "startDate": os.environ["START"],
    "endDate": os.environ["END"],
    "dimensions": os.environ["DIM"].split(","),
    "type": os.environ["TYPE"],
    "dataState": "final",
    "rowLimit": 25000,
}))
')"

SITE_ENC="$(SITE="$GSC_SITE_URL" python3 -c '
import os, urllib.parse
print(urllib.parse.quote(os.environ["SITE"], safe=""))
')"

RESP="$(curl -s -m 60 -X POST "$API/sites/$SITE_ENC/searchAnalytics/query" \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d "$BODY")"

if [ "$RAW" = 1 ]; then echo "$RESP"; exit 0; fi

RESP="$RESP" DIM="$DIM" LIMIT="$LIMIT" START="$START" END="$END" SITE="$GSC_SITE_URL" python3 -c '
import json, os, sys

d = json.loads(os.environ["RESP"])
if "error" in d:
    e = d["error"]
    sys.exit("API error %s: %s" % (e.get("code"), e.get("message")))

rows = d.get("rows", [])
dims = os.environ["DIM"].split(",")
limit = int(os.environ["LIMIT"])

print("%s — %s .. %s — %d rows (showing %d)\n" % (
    os.environ["SITE"], os.environ["START"], os.environ["END"],
    len(rows), min(limit, len(rows))))
if len(rows) == 25000:
    print("!! hit the 25000-row cap — this view is truncated\n")
if not rows:
    print("(no rows)"); raise SystemExit

width = 58
head = "  ".join(["%-*s" % (width, "/".join(dims))] + ["%7s" % h for h in ("clicks", "impr", "ctr", "pos")])
print(head); print("-" * len(head))
for r in rows[:limit]:
    key = " | ".join(r["keys"])
    if len(key) > width: key = key[:width - 1] + "…"
    print("  ".join(["%-*s" % (width, key),
                     "%7d" % r["clicks"], "%7d" % r["impressions"],
                     "%6.1f%%" % (r["ctr"] * 100), "%7.1f" % r["position"]]))

tot_c = sum(r["clicks"] for r in rows)
tot_i = sum(r["impressions"] for r in rows)
print("-" * len(head))
print("%-*s  %7d  %7d" % (width, "total (visible rows)", tot_c, tot_i))
print("\nnote: anonymized queries are excluded from rows above but counted in GSC totals.")
'
