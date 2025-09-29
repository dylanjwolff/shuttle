//! Constant stepped time model

use std::{
    cmp::{max, Reverse},
    collections::BinaryHeap,
    task::Waker,
};

use tracing::{debug, warn};

use crate::{current::TaskId, runtime::execution::ExecutionState};

use super::{Duration, Instant, TimeDistribution, TimeModel};

/// A time model where time advances by a constant amount for each step
#[derive(Clone, Debug)]
pub struct ConstantSteppedTimeModel {
    distribution: ConstantTimeDistribution,
    current_step_size: std::time::Duration,
    current_time_elapsed: std::time::Duration,
    waiters: BinaryHeap<Reverse<(std::time::Duration, DeadlineWaker)>>,
}

unsafe impl Send for ConstantSteppedTimeModel {}

impl ConstantSteppedTimeModel {
    /// Create a ConstantSteppedTimeModel
    pub fn new(distribution: ConstantTimeDistribution) -> Self {
        Self {
            distribution,
            current_step_size: distribution.sample(),
            current_time_elapsed: std::time::Duration::from_secs(0),
            waiters: BinaryHeap::new(),
        }
    }

    fn unblock_expired(&mut self) {
        while let Some(waker) = self.waiters.peek().and_then(|Reverse((t, waker))| {
            if *t <= self.current_time_elapsed {
                Some(waker.clone())
            } else {
                None
            }
        }) {
            _ = self.waiters.pop();
            match waker {
                DeadlineWaker::SyncSleep(id) => ExecutionState::with(|state| state.get_mut(id).unblock()),
                DeadlineWaker::AsyncWaker(_, w) => w.wake(),
            }
        }
    }
}

#[derive(Debug, Clone)]
enum DeadlineWaker {
    SyncSleep(TaskId),
    AsyncWaker(TaskId, Waker),
}

impl PartialEq for DeadlineWaker {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DeadlineWaker::SyncSleep(a), DeadlineWaker::SyncSleep(b)) => a == b,
            (DeadlineWaker::AsyncWaker(a, _), DeadlineWaker::AsyncWaker(b, _)) => a == b,
            _ => false,
        }
    }
}

impl Eq for DeadlineWaker {}

impl PartialOrd for DeadlineWaker {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DeadlineWaker {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (DeadlineWaker::SyncSleep(a), DeadlineWaker::SyncSleep(b))
            | (DeadlineWaker::AsyncWaker(a, _), DeadlineWaker::AsyncWaker(b, _))
            | (DeadlineWaker::SyncSleep(a), DeadlineWaker::AsyncWaker(b, _))
            | (DeadlineWaker::AsyncWaker(a, _), DeadlineWaker::SyncSleep(b)) => a.cmp(b),
        }
    }
}

impl TimeModel for ConstantSteppedTimeModel {
    fn pause(&mut self) {
        warn!("Pausing stepped model has no effect")
    }

    fn resume(&mut self) {
        warn!("Resuming stepped model has no effect")
    }

    fn sleep(&mut self, duration: Duration) {
        debug!("sleep");
        let duration = duration.unwrap_std();

        if duration == std::time::Duration::from_secs(0) {
            return;
        }
        let wake_time = self.current_time_elapsed + duration;
        let item = (
            wake_time,
            DeadlineWaker::SyncSleep(ExecutionState::with(|s| s.current().id())),
        );
        self.waiters.push(Reverse(item));
        ExecutionState::with(|s| s.current_mut().block(false));
    }

    fn step(&mut self) {
        debug!("step");
        self.current_time_elapsed += self.current_step_size;
        self.unblock_expired();
    }

    fn reset(&mut self) {
        self.current_step_size = self.distribution.sample();
        self.current_time_elapsed = std::time::Duration::from_secs(0);
        self.waiters.clear();
    }

    fn instant(&self) -> Instant {
        Instant::Simulated(self.current_time_elapsed)
    }

    fn wake_next(&mut self) -> bool {
        println!("wake next {:?}", self.waiters.peek());
        println!("wake next {:?}", self.waiters);
        if self.waiters.len() == 0 {
            return false;
        }
        if let Some(Reverse((time, _))) = self.waiters.peek() {
            self.current_time_elapsed = max(self.current_time_elapsed, *time);
        }
        self.unblock_expired();
        true
    }

    fn advance(&mut self, dur: Duration) {
        self.current_time_elapsed += dur.unwrap_std();
    }

    fn poll_timeout_is_expired(&mut self, deadline: Instant, waker: Option<Waker>) -> bool {
        let deadline = deadline.unwrap_simulated();
        if deadline <= self.current_time_elapsed {
            return true;
        }

        if let Some(waker) = waker {
            let id = ExecutionState::with(|s| s.current().id());
            let item = (deadline, DeadlineWaker::AsyncWaker(id, waker));
            self.waiters.push(Reverse(item));
        }
        false
    }
}

/// A constant distrubution; each sample returns the same time
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct ConstantTimeDistribution {
    /// The time that will be returned on sampling
    pub time: std::time::Duration,
}

impl ConstantTimeDistribution {
    /// Create a new constant time distribution
    pub fn new(time: std::time::Duration) -> Self {
        Self { time }
    }
}

impl TimeDistribution<std::time::Duration> for ConstantTimeDistribution {
    fn sample(&self) -> std::time::Duration {
        self.time
    }
}
