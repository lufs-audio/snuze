#!/bin/bash
# Configurable fake tmux binary for unit testing TmuxAdapter.
#
# When $FAKE_TMUX_LOG is set, every argv element is appended to that file, one
# per line, so tests can assert the exact invocation without a live tmux server.
# NOTE: like tests/fixtures/fake_herdr.sh, this proves argv construction and
# error mapping given known tmux outputs; it is not a substitute for a live tmux.

if [ -n "$FAKE_TMUX_LOG" ]; then
    for arg in "$@"; do
        printf '%s\n' "$arg" >>"$FAKE_TMUX_LOG"
    done
fi

case "$FAKE_TMUX_BEHAVIOR" in
    "" | "delivered" | "success")
        exit 0
        ;;
    "pane_not_found")
        echo "can't find pane: %99" >&2
        exit 1
        ;;
    "session_not_found")
        echo "can't find session: work" >&2
        exit 1
        ;;
    "no_server")
        echo "no server running on /tmp/tmux-1000/default" >&2
        exit 1
        ;;
    "generic_error")
        echo "tmux: unexpected failure" >&2
        exit 1
        ;;
    "timeout")
        sleep 5
        exit 0
        ;;
    *)
        echo "tmux: unknown fake behavior" >&2
        exit 1
        ;;
esac
