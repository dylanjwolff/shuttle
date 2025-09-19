//! Time
//!
//! Timing primitives allow Shuttle tests to interact with wall-clock time in a deterministic manner

use std::cmp::Ordering;
use std::future::Future;
use std::ops::{Add, Mul};
use std::{cell::RefCell, rc::Rc};

use std::pin::Pin;

use pin_project::pin_project;
use std::task::{Context, Poll};

use crate::runtime::execution::ExecutionState;

use crate::runtime::thread;

pub mod constant_stepped;

/// A distribution of times which can be sampled
pub trait TimeDistribution<D> {
    /// Sample a duration from the given distribution
    fn sample(&self) -> D;
}

/// The trait implemented by each TimeModel
pub trait TimeModel: std::fmt::Debug {
    /// sleep
    fn sleep(&mut self, duration: Duration);
    /// wake the next sleeping task if all tasks are blocked
    fn wake_next(&mut self);
    /// reset
    fn reset(&mut self);
    /// step
    fn step(&mut self);
    /// instant
    fn instant(&self) -> Instant;
    /// pause
    fn pause(&mut self);
    /// resume
    fn resume(&mut self);
}

fn get_time_model() -> Rc<RefCell<dyn TimeModel>> {
    ExecutionState::with(|s| Rc::clone(&s.time_model))
}

/// A Shuttle duration
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Duration {
    /// A concrete duration value
    Std(std::time::Duration),
}

impl Duration {
    /// Creates a new Duration from the specified number of seconds.
    pub fn from_secs(secs: u64) -> Self {
        Duration::Std(std::time::Duration::from_secs(secs))
    }

    /// Creates a new Duration from the specified number of milliseconds.
    pub fn from_millis(millis: u64) -> Self {
        Duration::Std(std::time::Duration::from_millis(millis))
    }

    /// Creates a new Duration from the specified number of microseconds.
    pub fn from_micros(micros: u64) -> Self {
        Duration::Std(std::time::Duration::from_micros(micros))
    }

    /// Creates a new Duration from the specified number of nanoseconds.
    pub fn from_nanos(nanos: u64) -> Self {
        Duration::Std(std::time::Duration::from_nanos(nanos))
    }

    /// Returns the total number of nanoseconds contained by this Duration.
    pub fn as_nanos(&self) -> u128 {
        match self {
            Duration::Std(d) => d.as_nanos(),
        }
    }

    /// Returns the total number of microseconds contained by this Duration.
    pub fn as_micros(&self) -> u128 {
        self.as_nanos() / 1000
    }

    /// Returns the total number of milliseconds contained by this Duration.
    pub fn as_millis(&self) -> u128 {
        self.as_micros() / 1000
    }

    ///  Checked Duration addition. Computes self + other, returning None if overflow occurred.
    pub fn checked_add(&self, other: Duration) -> Option<Self> {
        match (self, other) {
            (Duration::Std(a), Duration::Std(b)) => a.checked_add(b).map(Duration::Std),
        }
    }

    ///  Checked Duration multiplication. Computes self * other, returning None if overflow occurred.
    pub fn checked_mul(&self, b: u32) -> Option<Self> {
        match self {
            Duration::Std(a) => a.checked_mul(b).map(Duration::Std),
        }
    }

    pub(crate) fn unwrap_std(self) -> std::time::Duration {
        match self {
            Duration::Std(d) => d,
        }
    }
}

impl Ord for Duration {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Duration::Std(a), Duration::Std(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Duration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Add for Duration {
    type Output = Duration;

    fn add(self, other: Self) -> Self {
        self.checked_add(other).unwrap()
    }
}

impl Mul<u32> for Duration {
    type Output = Duration;

    fn mul(self, other: u32) -> Self {
        self.checked_mul(other).unwrap()
    }
}

impl Mul<Duration> for u32 {
    type Output = Duration;

    fn mul(self, other: Duration) -> Duration {
        other.checked_mul(self).unwrap()
    }
}

/// A Shuttle Instant
#[derive(Clone, Copy, Debug)]
pub enum Instant {
    /// Deterministically simulated clock time represented by a Duration from the start of the test
    Simulated(std::time::Duration),
}

impl Instant {
    /// Returns an instant corresponding to “now”.
    pub fn now() -> Self {
        get_time_model().borrow().instant()
    }

    /// Returns the amount of time elapsed from another instant to this one, or None if that instant is later than this one.
    /// Due to monotonicity bugs, even under correct logical ordering of the passed Instants, this method can return None.
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        match (self, earlier) {
            (Instant::Simulated(a), Instant::Simulated(b)) => a.checked_sub(b).map(Duration::Std),
        }
    }

    /// Returns Some(t) where t is the time self + duration if t can be represented as Instant (which means it’s inside the bounds
    /// of the underlying data structure), None otherwise.
    pub fn checked_add(&self, duration: Duration) -> Option<Self> {
        match (self, duration) {
            (Instant::Simulated(a), Duration::Std(b)) => a.checked_add(b).map(Instant::Simulated),
        }
    }

    /// Returns the amount of time elapsed since this instant.
    /// Previous Rust versions panicked when the current time was earlier than self. Currently this method returns a Duration of
    /// zero in that case. Future versions may reintroduce the panic.
    pub fn elapsed(&self) -> Duration {
        Instant::now()
            .checked_duration_since(*self)
            .unwrap_or(Duration::from_secs(0))
    }
}

/// Puts the current thread to sleep
/// Behavior of this function depends on the TimeModel provided to Shuttle
pub fn sleep(dur: Duration) {
    ExecutionState::with(|s| Rc::clone(&s.time_model))
        .borrow_mut()
        .sleep(dur);
    thread::switch();
}

/// Returns a future which sleeps until the duration has elapsed
pub async fn tokio_sleep(dur: Duration) {
    sleep(dur);
}

/// Returns a future which sleeps until the deadline is reached
pub async fn tokio_sleep_until(deadline: Instant) {
    if let Some(dur) = deadline.checked_duration_since(Instant::now()) {
        sleep(dur);
    }
}

/// Tokio interval
pub fn tokio_interval(dur: Duration) -> Interval {
    Interval {
        start: None,
        ticks: 0,
        duration: dur,
    }
}

/// Tokio interval
pub fn tokio_interval_at(start: Instant, period: Duration) -> Interval {
    Interval {
        start: Some(start),
        ticks: 0,
        duration: period,
    }
}

/// Timeout a future
#[pin_project]
#[derive(Debug)]
pub struct Interval {
    start: Option<Instant>,
    ticks: usize,
    duration: Duration,
}

impl Interval {
    /// tick
    pub async fn tick(&mut self) -> Instant {
        let ret = if let Some(start) = self.start {
            let mut total_duration = Duration::from_millis(0);
            // TODO: switch to multiply
            for _ in 1..=self.ticks {
                total_duration = total_duration.checked_add(self.duration).unwrap();
            }
            let end = start.checked_add(total_duration).unwrap();
            let now = Instant::now();
            if let Some(sleep_time) = end.checked_duration_since(now) {
                sleep(sleep_time);
            }
            end
        } else {
            let now = Instant::now();
            self.start = Some(now);
            now
        };
        self.ticks += 1;
        ret
    }
}

/// Timeout a future
pub fn tokio_timeout<F>(f: F, d: Duration) -> Timeout<F>
where
    F: Future,
{
    Timeout {
        start: None,
        duration: d,
        future: f,
    }
}

/// Timeout a future
#[pin_project]
#[derive(Debug)]
pub struct Timeout<F>
where
    F: Future,
{
    start: Option<Instant>,
    duration: Duration,
    #[pin]
    future: F,
}

/// Elapsed time error variant
#[derive(Debug, Clone, Copy)]
pub struct Elapsed;

impl<F> Future for Timeout<F>
where
    F: Future,
{
    type Output = std::result::Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let start = this.start.get_or_insert_with(Instant::now);
        if start.elapsed() > *this.duration {
            return Poll::Ready(Err(Elapsed));
        }

        match this.future.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(x) => {
                if start.elapsed() > *this.duration {
                    Poll::Ready(Err(Elapsed))
                } else {
                    Poll::Ready(Ok(x))
                }
            }
        }
    }
}
