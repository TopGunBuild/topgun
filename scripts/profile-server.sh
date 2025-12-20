#!/bin/bash
# Server Profiling Script
# Usage: ./scripts/profile-server.sh [mode]
# Modes: cpu, flame, bubble, inspect

set -e

MODE="${1:-cpu}"
SERVER_SCRIPT="packages/server/dist/index.js"
PROFILE_DIR="profiles"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}TopGun Server Profiler${NC}"
echo "========================"

# Ensure server is built
if [ ! -f "$SERVER_SCRIPT" ]; then
    echo -e "${YELLOW}Building server...${NC}"
    pnpm --filter @topgunbuild/server build
fi

# Create profiles directory
mkdir -p "$PROFILE_DIR"

case "$MODE" in
    cpu)
        echo -e "${GREEN}Starting CPU profiling...${NC}"
        echo "Run your benchmark, then stop server with Ctrl+C"
        echo "Profile will be saved to $PROFILE_DIR/"
        node --prof "$SERVER_SCRIPT"

        # Process the profile
        ISOLATE_FILE=$(ls -t isolate-*.log 2>/dev/null | head -1)
        if [ -n "$ISOLATE_FILE" ]; then
            echo -e "${GREEN}Processing profile...${NC}"
            node --prof-process "$ISOLATE_FILE" > "$PROFILE_DIR/cpu-profile-$(date +%Y%m%d-%H%M%S).txt"
            mv "$ISOLATE_FILE" "$PROFILE_DIR/"
            echo -e "${GREEN}Profile saved to $PROFILE_DIR/${NC}"
        fi
        ;;

    flame)
        echo -e "${GREEN}Starting Flame Graph profiling with Clinic.js...${NC}"
        echo "This will automatically generate a flame graph"
        npx clinic flame -- node "$SERVER_SCRIPT"
        ;;

    bubble)
        echo -e "${GREEN}Starting Bubble profiling with Clinic.js...${NC}"
        echo "This analyzes async operations"
        npx clinic bubbleprof -- node "$SERVER_SCRIPT"
        ;;

    doctor)
        echo -e "${GREEN}Starting Clinic Doctor...${NC}"
        echo "This provides overall health analysis"
        npx clinic doctor -- node "$SERVER_SCRIPT"
        ;;

    inspect)
        echo -e "${GREEN}Starting with Chrome DevTools inspector...${NC}"
        echo "Open chrome://inspect in Chrome browser"
        echo "Click 'inspect' under Remote Target"
        node --inspect "$SERVER_SCRIPT"
        ;;

    heap)
        echo -e "${GREEN}Starting with heap snapshot support...${NC}"
        echo "Send SIGUSR2 to take heap snapshot: kill -SIGUSR2 <pid>"
        node --inspect --expose-gc "$SERVER_SCRIPT"
        ;;

    *)
        echo -e "${RED}Unknown mode: $MODE${NC}"
        echo ""
        echo "Available modes:"
        echo "  cpu     - V8 CPU profiler (generates text report)"
        echo "  flame   - Clinic.js flame graph (visual)"
        echo "  bubble  - Clinic.js async analysis (visual)"
        echo "  doctor  - Clinic.js overall health (visual)"
        echo "  inspect - Chrome DevTools inspector"
        echo "  heap    - Heap snapshot support"
        exit 1
        ;;
esac
