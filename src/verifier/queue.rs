// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded verification queue with load shedding (DESIGN.md §6.4).
//!
//! Proof verification is expensive. Under sustained overload it must not block the FIFO
//! plan-processing path. A full queue sheds the new item at once and counts the shed.
//! Shedding is visible to the caller, so a relayer can retry later instead of waiting.

use std::collections::VecDeque;

/// Returned when an item is shed because the queue is full.
#[derive(Debug, PartialEq, Eq)]
pub struct Shed;

#[derive(Debug)]
pub struct VerificationQueue<T> {
    capacity: usize,
    items: VecDeque<T>,
    shed: u64,
}

impl<T> VerificationQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::with_capacity(capacity),
            shed: 0,
        }
    }

    /// Enqueue `item`, or shed it when the queue is full. Never blocks.
    pub fn try_push(&mut self, item: T) -> Result<(), Shed> {
        if self.items.len() >= self.capacity {
            self.shed += 1;
            return Err(Shed);
        }
        self.items.push_back(item);
        Ok(())
    }

    /// Dequeue the oldest item.
    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total items shed since the queue was created.
    pub fn shed_count(&self) -> u64 {
        self.shed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_arrival_order() {
        let mut q = VerificationQueue::new(3);
        q.try_push(1).unwrap();
        q.try_push(2).unwrap();
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn full_queue_sheds_and_counts() {
        let mut q = VerificationQueue::new(2);
        q.try_push("a").unwrap();
        q.try_push("b").unwrap();
        assert_eq!(q.try_push("c"), Err(Shed));
        assert_eq!(q.try_push("d"), Err(Shed));
        assert_eq!(q.len(), 2);
        assert_eq!(q.shed_count(), 2);
    }

    #[test]
    fn popping_frees_capacity() {
        let mut q = VerificationQueue::new(1);
        q.try_push(1).unwrap();
        assert!(q.try_push(2).is_err());
        q.pop();
        assert!(q.try_push(3).is_ok());
        assert_eq!(q.pop(), Some(3));
        assert!(q.is_empty());
    }

    #[test]
    fn zero_capacity_sheds_everything() {
        let mut q: VerificationQueue<u8> = VerificationQueue::new(0);
        assert_eq!(q.try_push(1), Err(Shed));
        assert_eq!(q.shed_count(), 1);
    }
}
