#!/bin/sh
set -e

# Start Rust server in background on port 8080
PORT=8080 /usr/local/bin/topgun-server &

# Wait briefly for server to bind
sleep 1

# Start nginx in foreground (keeps container alive)
nginx -g 'daemon off;'
