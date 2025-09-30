//! Frozen time with triggers

use std::{cmp::Reverse, collections::HashSet, task::Waker};

use tracing::warn;

use crate::{
    current::{with_labels_for_task, Labels, TaskId},
    runtime::execution::ExecutionState,
};

use super::{
    constant_stepped::ConstantSteppedTimeModel, constant_stepped::ConstantTimeDistribution, Duration, Instant,
    TimeModel,
};

/// A time model where time does not advance unless forced
#[derive(Clone, Debug)]
pub struct FrozenTimeModel {
    inner: ConstantSteppedTimeModel,
    expired: HashSet<(std::time::Duration, TaskId)>,
}

unsafe impl Send for FrozenTimeModel {}

impl FrozenTimeModel {
    /// Create a new Frozen time model
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for FrozenTimeModel {
    fn default() -> Self {
        Self {
            inner: ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::ZERO)),
            expired: HashSet::new(),
        }
    }
}

impl TimeModel for FrozenTimeModel {
    fn pause(&mut self) {
        warn!("Pausing frozen model has no effect")
    }

    fn resume(&mut self) {
        warn!("Resuming frozen model has no effect")
    }

    fn step(&mut self) {}

    fn reset(&mut self) {
        self.inner.reset();
        self.expired.clear();
    }

    fn instant(&self) -> Instant {
        self.inner.instant()
    }

    fn wake_next(&mut self) -> bool {
        self.inner.wake_next()
    }

    fn advance(&mut self, dur: Duration) {
        self.inner.advance(dur);
    }

    fn register_sleep(&mut self, deadline: Instant, waker: Option<Waker>) -> bool {
        let task_id = ExecutionState::me();
        let std_deadline = deadline.unwrap_simulated();
        if !self.expired.contains(&(std_deadline, task_id)) {
            self.inner.register_sleep(deadline, waker)
        } else {
            true
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn trigger_timeouts(&mut self, trigger: Box<dyn Fn(&Labels) -> bool>) {
        let mut to_wake = Vec::new();
        for Reverse((deadline, task_id)) in self.inner.get_waiters() {
            with_labels_for_task(*task_id, |labels| {
                if trigger(labels) {
                    to_wake.push((*deadline, *task_id));
                }
            })
        }
        for (deadline, task_id) in to_wake {
            self.inner.wake_frozen(deadline, task_id);
            self.expired.insert((deadline, task_id));
        }
    }
}
