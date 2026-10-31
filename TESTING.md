# `snuze` — Live Manual Testing Runbook

This runbook documents manual verification procedures for scenarios requiring a live, interactive terminal multiplexer (`herdr` or `tmux`) that cannot be executed in headless continuous integration (CI).

---

## 1. Prerequisites

1. **Terminal Multiplexer:** A running `herdr` session with at least one active agent pane (e.g., `w1:p1`) and one shell pane (e.g., `w1:p2`), and/or a running `tmux` session.
   ```bash
   herdr pane list
   tmux list-panes
   ```
2. **Environment Variables:**
   - `HERDR_PANE_ID`: Set automatically inside Herdr panes, or export manually for testing.
   - `TMUX` / `TMUX_PANE`: Set automatically inside tmux panes; used for delivery-target auto-detection.
3. **Compiled Binary:**
   ```bash
   cargo build --release
   export PATH="$(pwd)/target/release:$PATH"
   ```

---

## 2. Test Scenarios

### Scenario A: Real-Time Prompt Injection into Live Agent Pane

1. **Identify Target Agent Pane:**
   ```bash
   herdr pane list
   # Example output shows pane 'w1:p1' running OpenCode / pi agent
   ```
2. **Schedule Prompt Alarm (10 seconds):**
   ```bash
   snuze set --after 10s --pane w1:p1 --prompt "Wake up! Snuze live verification test." --label "live-prompt-test" --json
   ```
   Note the `id` from the output JSON.
3. **Verify Pending State:**
   ```bash
   snuze status <ALARM_ID>
   ```
4. **Observe Target Pane:**
   - Watch the target pane in the Herdr interface.
   - After 10 seconds, confirm that `herdr agent prompt` successfully injects the message.
   - Confirm that the agent receives the prompt and starts generating a turn.
5. **Verify Final State:**
   ```bash
   snuze status <ALARM_ID>
   # Output must show: Status: Fired
   ```

---

### Scenario B: Command Execution in Shell Pane

1. **Schedule Command Alarm (5 seconds):**
   ```bash
   snuze set --after 5s --pane w1:p2 --command "echo 'snuze-command-verified' > /tmp/snuze_cmd_test.txt" --label "live-cmd-test"
   ```
2. **Wait and Verify Execution:**
   ```bash
   sleep 6
   cat /tmp/snuze_cmd_test.txt
   # Should print: snuze-command-verified
   ```

---

### Scenario C: tmux Delivery and Target Auto-Detection

1. **Explicit tmux Target:**
   ```bash
   snuze set --after 10s --tmux %0 --prompt "tmux delivery verified" --json
   ```
   Confirm the prompt appears in the current tmux pane after 10 seconds.
2. **Auto-Detected Target (inside tmux, no target flag):**
   ```bash
   snuze set --after 10s --prompt "auto-detected tmux delivery" --json
   snuze status <ALARM_ID> --json
   # Confirm "target": {"type":"tmux","target":"%N"} matches $TMUX_PANE
   ```
3. **Unresolvable Target (outside Herdr and tmux):**
   ```bash
   env -u HERDR_ENV -u TMUX snuze set --after 10s --prompt "no target" --json
   # Must exit 2 (USAGE) with an actionable message
   ```

---

### Scenario D: Process-Exit Trigger (`--pid`)

1. **Start a Background Process:**
   ```bash
   sleep 30 &
   PID=$!
   ```
2. **Schedule the Alarm:**
   ```bash
   snuze set --pid "$PID" --prompt "Process $PID exited" --tmux %0 --json
   ```
3. **Terminate the Process and Observe:**
   ```bash
   kill "$PID"
   # The prompt must be delivered within the poll backoff window (<= ~1s initially).
   snuze status <ALARM_ID> --json
   # Status: fired
   ```
4. **Verify the Preflight Rejection:**
   ```bash
   snuze set --pid 999999 --prompt "never" --tmux %0 --json
   # Must exit 3 (TargetUnavailable) with "process 999999 is not running"
   ```

---

### Scenario E: File-Artifact Trigger (`--watch-file`)

1. **Schedule an Existence Watch:**
   ```bash
   rm -f /tmp/snuze_take.json
   snuze set --watch-file /tmp/snuze_take.json --prompt "take.json ready" --tmux %0 --json
   ```
2. **Write the Artifact from Another Shell:**
   ```bash
   echo '{}' > /tmp/snuze_take.json
   ```
   Confirm delivery within the poll backoff window.
3. **Modified-Mode and Wildcards:**
   ```bash
   snuze set --watch-file '/tmp/snuze-take-*.json' --watch-mode modified --prompt "new take" --tmux %0 --json
   echo '{}' > /tmp/snuze-take-001.json
   ```

---

### Scenario F: Blocking `snuze wait`

1. **Schedule and Block:**
   ```bash
   ID=$(snuze set --target wait --after 20s --json | jq -r .data.id)
   snuze wait "$ID" --timeout 1m --json
   echo "exit code: $?"
   # Must block ~20s, then print a success envelope and exit 0.
   ```
2. **Non-Fired Terminal State:**
   ```bash
   ID=$(snuze set --target wait --after 10m --json | jq -r .data.id)
   (sleep 1; snuze cancel "$ID" --json) &
   snuze wait "$ID" --timeout 1m --json
   echo "exit code: $?"
   # Must exit 5 (ContractViolated): the alarm was canceled, not fired.
   ```
3. **Concurrency:** while a `snuze wait` is blocked, confirm `snuze list --json` and `snuze doctor` still respond immediately.

---

### Scenario G: Live Crash Recovery (SIGKILL Fault Injection)

1. **Schedule a 5-Minute Alarm:**
   ```bash
   snuze set --after 5m --pane w1:p1 --prompt "Survived daemon crash!" --label "crash-test"
   ```
2. **Kill Daemon Process Uncooperatively (`kill -9`):**
   ```bash
   STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/snuze"
   kill -9 $(cat "$STATE_DIR/daemon.pid")
   ```
3. **Restart Daemon:**
   ```bash
   snuze daemon start
   ```
4. **Verify Alarm Re-Armed:**
   ```bash
   snuze list --status pending
   # Confirms crash-test alarm is present and still pending with original deadline
   ```

---

### Scenario H: Diagnostic Self-Check (`doctor`)

1. **Run Diagnostic Check:**
   ```bash
   snuze doctor
   ```
2. **Expected Healthy Output:**
   ```text
   Herdr Binary: herdr <version>
   Herdr Socket: reachable
   Daemon Running: running
   State Dir Writable: yes
   Pending Alarms: <N>
   Failed Alarms: <N>
   Missed Alarms: <N>
   Corrupt Alarms: 0
   ```

---

### Scenario I: Herdr Event Subscription Push Wake-up (`--herdr-pane`)

1. **Schedule Watch on Sibling Pane:**
   ```bash
   snuze set --herdr-pane w1:p2 --watch-for done,idle --timeout 15m --pane w1:p1 --prompt "Watched pane w1:p2 transitioned to {status}!" --json
   ```
2. **Trigger Lifecycle Transition in Pane `w1:p2`:**
   - Execute a task in `w1:p2` or send a command until the agent reaches `done`/`idle`.
3. **Verify Push Resolution:**
   - Observe immediate prompt injection in pane `w1:p1` without waiting for a poll interval.
   - Verify `snuze status <ID>` transitions to `fired`.

---

### Scenario J: External Signal Dispatch (`snuze signal`)

1. **Schedule Signal-Triggered Alarm:**
   ```bash
   snuze set --signal "deploy-complete-xyz" --timeout 1h --pane w1:p1 --prompt "Deployment xyz completed!" --json
   ```
2. **Simulate External Webhook / Dispatcher Signal:**
   ```bash
   snuze signal "deploy-complete-xyz" --json
   # Output: {"status":"success","data":{"alarm_ids":["..."],"matched":1}}
   ```
3. **Verify Delivery:**
   - Confirm delivery to `w1:p1` and terminal status `fired`.

---

### Scenario K: Local Event Outbox Emission & Draining (`events drain`)

1. **Inspect Drained Events:**
   ```bash
   snuze events drain --json
   ```
   Confirm all terminal events (`fired`, `failed`, `canceled`, `missed`) are returned as valid JSONL entries with timestamps, hostnames, and trigger kinds.
2. **Confirm Idempotent Drain:**
   ```bash
   snuze events drain --json
   # Subsequent call returns empty array []
   ```
