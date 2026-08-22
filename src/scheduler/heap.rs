use crate::kernel::Alarm;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Internal wrapper for heap elements to invert ordering (earliest deadline first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeapEntry {
    /// Inner alarm.
    pub alarm: Alarm,
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Rust's BinaryHeap is a max-heap; invert deadline comparison to pop earliest first.
        other
            .alarm
            .deadline
            .cmp(&self.alarm.deadline)
            .then_with(|| other.alarm.id.cmp(&self.alarm.id))
    }
}

/// Min-heap of pending alarms ordered by deadline.
#[derive(Debug, Default)]
pub struct AlarmHeap {
    heap: BinaryHeap<HeapEntry>,
}

impl AlarmHeap {
    /// Creates an empty `AlarmHeap`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    /// Pushes an alarm into the heap.
    pub fn push(&mut self, alarm: Alarm) {
        self.heap.push(HeapEntry { alarm });
    }

    /// Returns a reference to the earliest due alarm without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<&Alarm> {
        self.heap.peek().map(|e| &e.alarm)
    }

    /// Removes and returns the earliest due alarm from the heap.
    pub fn pop(&mut self) -> Option<Alarm> {
        self.heap.pop().map(|e| e.alarm)
    }

    /// Cancels and removes an alarm by ID. Returns `true` if found and removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let old_len = self.heap.len();
        self.heap.retain(|e| e.alarm.id != id);
        self.heap.len() < old_len
    }

    /// Returns the number of alarms in the heap.
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` if the heap contains no alarms.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}
