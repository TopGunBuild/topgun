#!/bin/bash
#
# A/B Performance Testing Script
#
# Runs k6 throughput tests against different server configurations
# and compares results.
#
# Usage:
#   ./scripts/perf-ab-test.sh [profiles...]
#
# Examples:
#   ./scripts/perf-ab-test.sh                    # Run all profiles
#   ./scripts/perf-ab-test.sh baseline low-delay # Run specific profiles
#   ./scripts/perf-ab-test.sh --quick            # Quick 30s test per profile
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RESULTS_DIR="$PROJECT_DIR/tests/k6/results/ab-test-$(date +%Y%m%d-%H%M%S)"
SERVER_STARTUP_WAIT=3
SERVER_COOLDOWN=2
K6_BINARY="$PROJECT_DIR/bin/k6"

# Default profiles to test
ALL_PROFILES="baseline low-delay large-batch no-coalescing aggressive"
PROFILES_TO_TEST=""
QUICK_MODE=false

# Parse arguments
for arg in "$@"; do
    case $arg in
        --quick)
            QUICK_MODE=true
            ;;
        *)
            PROFILES_TO_TEST="$PROFILES_TO_TEST $arg"
            ;;
    esac
done

# Use all profiles if none specified
if [ -z "$PROFILES_TO_TEST" ]; then
    PROFILES_TO_TEST="$ALL_PROFILES"
fi

echo -e "${BLUE}"
echo "╔══════════════════════════════════════════════════════════════════╗"
echo "║               TOPGUN A/B PERFORMANCE TEST                        ║"
echo "╠══════════════════════════════════════════════════════════════════╣"
echo "║  Profiles: $(echo $PROFILES_TO_TEST | tr -s ' ')                 "
echo "║  Results:  $RESULTS_DIR"
if [ "$QUICK_MODE" = true ]; then
echo "║  Mode:     Quick (30s per profile)                               ║"
fi
echo "╚══════════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

# Check k6 binary
if [ ! -f "$K6_BINARY" ]; then
    echo -e "${YELLOW}k6 binary not found. Building with xk6-msgpack...${NC}"
    cd "$PROJECT_DIR"
    pnpm test:k6:build
fi

# Create results directory
mkdir -p "$RESULTS_DIR"

# Generate JWT token once
echo -e "${BLUE}Generating JWT token...${NC}"
JWT_TOKEN=$(node "$PROJECT_DIR/scripts/generate-k6-token.js")
export JWT_TOKEN

# Function to run a single profile test
run_profile_test() {
    local profile=$1
    local result_file="$RESULTS_DIR/$profile.json"

    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Testing profile: $profile${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════════${NC}"

    # Start server with profile
    echo -e "${BLUE}Starting server with profile: $profile${NC}"
    PERF_PROFILE="$profile" pnpm exec ts-node \
        -r tsconfig-paths/register \
        --project "$PROJECT_DIR/examples/tsconfig.json" \
        "$PROJECT_DIR/scripts/perf-ab-server.ts" &
    SERVER_PID=$!

    # Wait for server to start
    sleep $SERVER_STARTUP_WAIT

    # Verify server is running
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}Server failed to start for profile: $profile${NC}"
        return 1
    fi

    # Run k6 test
    echo -e "${BLUE}Running k6 throughput test...${NC}"

    local k6_scenario="$PROJECT_DIR/tests/k6/scenarios/throughput-test.js"

    if [ "$QUICK_MODE" = true ]; then
        # Quick mode: shorter test
        "$K6_BINARY" run \
            --out json="$result_file" \
            --duration 30s \
            --vus 100 \
            "$k6_scenario" 2>&1 | tee "$RESULTS_DIR/$profile.log"
    else
        # Full test with ramping
        "$K6_BINARY" run \
            --out json="$result_file" \
            "$k6_scenario" 2>&1 | tee "$RESULTS_DIR/$profile.log"
    fi

    # Stop server
    echo -e "${BLUE}Stopping server...${NC}"
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true

    # Cooldown between tests
    sleep $SERVER_COOLDOWN

    echo -e "${GREEN}Profile $profile completed${NC}"
}

# Run tests for each profile
for profile in $PROFILES_TO_TEST; do
    run_profile_test "$profile"
done

# Generate comparison report
echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                    COMPARISON REPORT                              ${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"

# Create summary file
SUMMARY_FILE="$RESULTS_DIR/summary.txt"

echo "TopGun A/B Performance Test Results" > "$SUMMARY_FILE"
echo "Date: $(date)" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"
echo "Profile              | Throughput (ops/s) | p50 (ms) | p95 (ms) | p99 (ms) | Errors" >> "$SUMMARY_FILE"
echo "---------------------|--------------------|---------:|---------:|---------:|-------" >> "$SUMMARY_FILE"

for profile in $PROFILES_TO_TEST; do
    log_file="$RESULTS_DIR/$profile.log"
    if [ -f "$log_file" ]; then
        # Extract metrics from log (adjust patterns based on actual output)
        throughput=$(grep -o 'Average Throughput:[[:space:]]*[0-9]*' "$log_file" | grep -o '[0-9]*' | tail -1 || echo "N/A")
        p50=$(grep -o 'p50:[[:space:]]*[0-9.]*' "$log_file" | grep -o '[0-9.]*' | tail -1 || echo "N/A")
        p95=$(grep -o 'p95:[[:space:]]*[0-9.]*' "$log_file" | grep -o '[0-9.]*' | tail -1 || echo "N/A")
        p99=$(grep -o 'p99:[[:space:]]*[0-9.]*' "$log_file" | grep -o '[0-9.]*' | tail -1 || echo "N/A")
        errors=$(grep -o 'Error Rate:[[:space:]]*[0-9.]*' "$log_file" | grep -o '[0-9.]*' | tail -1 || echo "N/A")

        printf "%-20s | %18s | %8s | %8s | %8s | %s\n" \
            "$profile" "$throughput" "$p50" "$p95" "$p99" "$errors%" | tee -a "$SUMMARY_FILE"
    fi
done

echo "" >> "$SUMMARY_FILE"
echo "Results saved to: $RESULTS_DIR" >> "$SUMMARY_FILE"

echo ""
echo -e "${GREEN}Results saved to: $RESULTS_DIR${NC}"
echo -e "${GREEN}Summary: $SUMMARY_FILE${NC}"
