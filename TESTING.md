# `snuze` — Live Manual Testing Runbook

This runbook documents manual verification procedures for scenarios requiring a live, interactive Herdr terminal environment and external Google Jules API access that cannot be executed in headless continuous integration (CI).

---

## 1. Prerequisites

1. **Herdr Multiplexer:** A running `herdr` session with at least one active agent pane (e.g., `w1:p1`) and one shell pane (e.g., `w1:p2`).
   ```bash
   herdr pane list
   ```
2. **Environment Variables:**
   - `HERDR_PANE_ID`: Set automatically inside Herdr panes, or export manually for testing.
   - `JULES_API_KEY`: Required for testing Google Jules condition-driven alarms.
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

### Scenario C: Condition-Driven Jules Wake-Up

1. **Verify Environment:**
   ```bash
   echo $JULES_API_KEY
   ```
2. **Schedule Jules Condition Alarm:**
   ```bash
   snuze set \
     --jules "<JULES_SESSION_ID>" \
     --condition any-completed \
     --timeout 15m \
     --poll-interval 15s \
     --pane w1:p1 \
     --prompt "Jules session {session_id} reached state {status}. PR: {pr_urls}" \
     --json
   ```
3. **Inspect Progress:**
   ```bash
   snuze status <ALARM_ID> --json
   # Confirm "last_known" array reflects real Jules API session status (e.g. IN_PROGRESS)
   ```
4. **Observe Wake-Up on Completion:**
   - When Jules completes or fails, confirm the template tokens `{session_id}`, `{status}`, and `{pr_urls}` interpolate with real values and deliver into pane `w1:p1`.

---

### Scenario D: Live Crash Recovery (SIGKILL Fault Injection)

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

### Scenario E: Diagnostic Self-Check (`doctor`)

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
