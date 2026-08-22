#!/bin/bash
# Configurable fake herdr binary for unit and integration testing of RealHerdrAdapter.
# NOTE: These tests prove error-mapping and timeout logic given known Herdr outputs;
# they are not a substitute for the live-Herdr test suite (SPEC §9).

case "$FAKE_HERDR_BEHAVIOR" in
    "delivered" | "success" | "pane_exists_true")
        if [ "$1" = "pane" ] && [ "$2" = "get" ]; then
            echo '{"pane_id":"'"$3"'","title":"test pane"}'
        else
            echo '{"status":"ok"}'
        fi
        exit 0
        ;;
    "agent_blocked")
        echo '{"error":{"code":"agent_blocked","message":"target agent is busy with another prompt"},"id":"cli:agent:prompt"}' >&2
        exit 1
        ;;
    "agent_prompt_stalled")
        echo '{"error":{"code":"agent_prompt_stalled","message":"no lifecycle change observed within 5000ms"},"id":"cli:agent:prompt"}' >&2
        exit 1
        ;;
    "pane_not_found")
        echo '{"error":{"code":"pane_not_found","message":"pane w1:p99 not found"},"id":"cli:pane:get"}' >&2
        exit 1
        ;;
    "pane_not_found_shorthand")
        echo '{"code":"not_found","message":"pane not found"}' >&2
        exit 1
        ;;
    "pane_not_found_string")
        echo "herdr error: pane w1:p99 not found" >&2
        exit 1
        ;;
    "syntax_error")
        echo "error: unexpected argument '--invalid-flag' found" >&2
        exit 2
        ;;
    "timeout")
        sleep 5
        exit 0
        ;;
    "generic_error")
        echo '{"error":{"code":"internal_error","message":"unexpected daemon crash"},"id":"cli:error"}' >&2
        exit 1
        ;;
    "unparseable_error")
        echo "A very long raw stderr error string that exceeds the maximum limit for failure reason fields. This is repeated to test that truncation works properly and bounding is enforced: A very long raw stderr error string that exceeds the maximum limit for failure reason fields." >&2
        exit 1
        ;;
    *)
        if [ "$1" = "--version" ] || [ "$1" = "-V" ]; then
            echo "herdr 0.1.0-fake"
            exit 0
        fi
        echo '{"error":{"code":"unknown","message":"unknown fake behavior"},"id":"cli:error"}' >&2
        exit 1
        ;;
esac
