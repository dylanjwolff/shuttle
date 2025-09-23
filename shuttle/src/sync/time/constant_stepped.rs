//! Constant stepped time model

use std::{
    cmp::{max, Reverse},
    collections::BinaryHeap,
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
    waiters: BinaryHeap<Reverse<(std::time::Duration, TaskId)>>,
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

    fn unblock_expired(&mut self, state: &mut ExecutionState) {
        while let Some(id) = self.waiters.peek().and_then(|Reverse((t, id))| {
            if *t <= self.current_time_elapsed {
                Some(*id)
            } else {
                None
            }
        }) {
            _ = self.waiters.pop();
            state.get_mut(id).unblock();
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
        let item = (wake_time, ExecutionState::with(|s| s.current().id()));
        self.waiters.push(Reverse(item));
        ExecutionState::with(|s| s.current_mut().block(false));
    }

    fn step(&mut self) {
        debug!("step");
        self.current_time_elapsed += self.current_step_size;
        ExecutionState::with(|s| self.unblock_expired(s));
    }

    fn reset(&mut self) {
        self.current_step_size = self.distribution.sample();
        self.current_time_elapsed = std::time::Duration::from_secs(0);
        self.waiters.clear();
    }

    fn instant(&self) -> Instant {
        Instant::Simulated(self.current_time_elapsed)
    }

    fn wake_next(&mut self) {
        debug!("wake next");
        if let Some(Reverse((time, _))) = self.waiters.peek() {
            self.current_time_elapsed = max(self.current_time_elapsed, *time);
        }

        ExecutionState::with(|s| self.unblock_expired(s));
    }

    fn advance(&mut self, dur: Duration) {
        self.current_time_elapsed += dur.unwrap_std();
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
