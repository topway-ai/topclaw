#!/bin/bash
# dev-entrypoint.sh — Docker dev-stage entrypoint
#
# Starts Xvfb (virtual X11 display) so computer_use / screenshot / app_launch
# work inside the container, then execs the real topclaw binary.
#
# The display number is taken from $XVFB_DISPLAY (default :99).
# If Xvfb is not installed (e.g. custom image without desktop helpers),
# the script falls through without error — desktop automation tools will
# report the usual "missing helpers" message at runtime.

set -euo pipefail

# Start Xvfb if available
if command -v Xvfb >/dev/null 2>&1; then
    DISPLAY="${XVFB_DISPLAY:-:99}"
    export DISPLAY

    # Create a lock dir to prevent Xvfb from complaining about an existing lock
    XDIR="/tmp/X${DISPLAY#*:}-lock"
    rm -f "$XDIR"

    # -ac disables X access control; safe inside an isolated container
    Xvfb "$DISPLAY" -screen 0 1280x1024x24 -ac +extension GLX +render -noreset &
    XVFB_PID=$!

    # Give Xvfb a moment to start
    sleep 0.5

    # Verify Xvfb is running
    if kill -0 "$XVFB_PID" 2>/dev/null; then
        echo "topclaw dev: Xvfb started on $DISPLAY (pid $XVFB_PID)"
    else
        echo "topclaw dev: Xvfb failed to start — desktop automation will be unavailable"
        unset DISPLAY
    fi
else
    echo "topclaw dev: Xvfb not installed — desktop automation will be unavailable"
fi

# Exec the real topclaw binary (passed as the image CMD or user override)
exec topclaw "$@"
