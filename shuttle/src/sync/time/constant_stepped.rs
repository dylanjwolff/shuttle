//! Constant stepped time model

use std::{
    cmp::{max, Reverse},
    collections::{BinaryHeap, HashMap},
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
    waiters: BinaryHeap<Reverse<(std::time::Duration, TaskId)>>,
    wakers: HashMap<(std::time::Duration, TaskId), Waker>,
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
            wakers: HashMap::new(),
        }
    }

    fn unblock_expired(&mut self) {
        while let Some(waker_key) = self.waiters.peek().and_then(|Reverse((t, task_id))| {
            if *t <= self.current_time_elapsed {
                Some((*t, *task_id))
            } else {
                None
            }
        }) {
            _ = self.waiters.pop();
            println!("remove {:?} from {:?}", waker_key, self.wakers);
            if let Some(waker) = self.wakers.remove(&waker_key) {
                waker.wake();
            }
        }
    }

    /// Get the currently sleeping tasks and deadlines. May contain duplicates
    pub fn get_waiters(&self) -> &[Reverse<(std::time::Duration, TaskId)>] {
        self.waiters.as_slice()
    }

    /// Manually wake a task without affecting the global clock
    pub fn wake_frozen(&mut self, deadline: std::time::Duration, task_id: TaskId) {
        println!("try wake frozen {:?} {:?}", deadline, task_id);
        if let Some(waker) = self.wakers.remove(&(deadline, task_id)) {
            println!("wake frozen {:?} {:?}", deadline, task_id);
            waker.wake();
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

    fn step(&mut self) {
        debug!("step");
        self.current_time_elapsed += self.current_step_size;
        self.unblock_expired();
    }

    fn reset(&mut self) {
        self.current_step_size = self.distribution.sample();
        self.current_time_elapsed = std::time::Duration::from_secs(0);
        self.waiters.clear();
        self.wakers.clear();
    }

    fn instant(&self) -> Instant {
        Instant::Simulated(self.current_time_elapsed)
    }

    fn wake_next(&mut self) -> bool {
        println!("wake next {:?}", self.waiters.peek());
        println!("wake next {:?}", self.waiters);
        if self.waiters.is_empty() {
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

    fn register_sleep(&mut self, deadline: Instant, waker: Option<Waker>) -> bool {
        let deadline = deadline.unwrap_simulated();
        if deadline <= self.current_time_elapsed {
            return true;
        }

        if let Some(waker) = waker {
            println!("register sleep {:?} {:?}", deadline, waker);
            let id = ExecutionState::with(|s| s.current().id());
            let item = (deadline, id);
            self.waiters.push(Reverse(item));
            self.wakers.insert(item, waker);
        }
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
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
