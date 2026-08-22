use crate::kernel::Alarm;

/// Outcome of attempting to deliver an alarm to a Herdr pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Alarm delivered successfully to target pane.
    Delivered,
    /// Target agent was blocked on another interactive input or state (`--prompt` only).
    AgentBlocked(String),
    /// Target agent prompt did not trigger an observed lifecycle change within the window (`--prompt` only).
    PromptStalled(String),
    /// Target pane or agent no longer resolves or does not exist.
    PaneGone(String),
    /// Herdr daemon was unavailable, command timed out, or invocation was malformed.
    HerdrUnavailable(String),
}

/// Interface contract for dispatching alarm payloads to Herdr.
pub trait HerdrAdapter: Send + Sync {
    /// Deliver an alarm payload to the target Herdr pane.
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome;
}
