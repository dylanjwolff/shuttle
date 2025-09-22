//! Time
//!
//! Timing primitives allow Shuttle tests to interact with wall-clock time in a deterministic manner

use std::cmp::Ordering;
use std::future::Future;
use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};
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
    /// The maximum duration.
    pub const MAX: Duration = Duration::Std(std::time::Duration::MAX);
    /// Zero duration.
    pub const ZERO: Duration = Duration::Std(std::time::Duration::ZERO);

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

    ///  Checked Duration subtraction. Computes self - other, returning None if other is greater than self.
    pub fn checked_sub(&self, other: Duration) -> Option<Self> {
        match (self, other) {
            (Duration::Std(a), Duration::Std(b)) => a.checked_sub(b).map(Duration::Std),
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

impl AddAssign for Duration {
    fn add_assign(&mut self, other: Self) {
        *self = self.checked_add(other).unwrap()
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, other: Self) {
        *self = self.checked_sub(other).unwrap()
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    pub fn checked_sub(&self, earlier: Instant) -> Option<Duration> {
        match (self, earlier) {
            (Instant::Simulated(a), Instant::Simulated(b)) => a.checked_sub(b).map(Duration::Std),
        }
    }

    /// Returns the amount of time elapsed from another instant to this one, or None if that instant is later than this one.
    /// Due to monotonicity bugs, even under correct logical ordering of the passed Instants, this method can return None.
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.checked_sub(earlier)
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

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, other: Duration) -> Instant {
        self.checked_add(other).unwrap()
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, other: Duration) {
        *self = self.checked_add(other).unwrap()
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;

    fn sub(self, earlier: Instant) -> Duration {
        self.checked_sub(earlier).unwrap()
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, other: Duration) {
        *self = *self - other
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;

    fn sub(self, other: Duration) -> Instant {
        match (self, other) {
            (Instant::Simulated(i), Duration::Std(d)) => Instant::Simulated(i - d),
        }
    }
}

impl Ord for Instant {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Instant::Simulated(a), Instant::Simulated(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
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
pub fn tokio_sleep(dur: Duration) -> Sleep {
    Sleep {
        deadline: Instant::now().checked_add(dur).unwrap(),
    }
}

/// Returns a future which sleeps until the deadline is reached
pub fn tokio_sleep_until(deadline: Instant) -> Sleep {
    Sleep { deadline }
}

/// Tokio interval
pub fn tokio_interval(dur: Duration) -> Interval {
    Interval {
        start: None,
        ticks: 0,
        period: dur,
    }
}

/// Tokio interval
pub fn tokio_interval_at(start: Instant, period: Duration) -> Interval {
    Interval {
        start: Some(start),
        ticks: 0,
        period,
    }
}

/// sleep
#[pin_project]
#[derive(Debug)]
pub struct Sleep {
    deadline: Instant,
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = Instant::now();
        if let Some(dur) = self.deadline.checked_duration_since(now) {
            sleep(dur);
            Poll::Ready(())
        } else {
            Poll::Ready(())
        }
    }
}

impl Sleep {
    /// Returns the instant at which the future will complete.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns `true` if `Sleep` has elapsed.
    ///
    /// A `Sleep` instance is elapsed when the requested duration has elapsed.
    pub fn is_elapsed(&self) -> bool {
        self.deadline.checked_duration_since(Instant::now()).is_none()
    }

    /// Resets the `Sleep` instance to a new deadline.
    pub fn reset(self: Pin<&mut Self>, deadline: Instant) {
        let me = self.project();
        *me.deadline = deadline;
    }
}

/// Timeout a future
#[pin_project]
#[derive(Debug)]
pub struct Interval {
    start: Option<Instant>,
    ticks: u32,
    period: Duration,
}

impl Interval {
    /// tick
    pub async fn tick(&mut self) -> Instant {
        self.tick_inner()
    }

    fn tick_inner(&mut self) -> Instant {
        let ret = if let Some(start) = self.start {
            let mut total_duration = Duration::from_millis(0);
            total_duration += self.period * self.ticks;
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

    /// poll tick
    pub fn poll_tick(&mut self, _cx: &mut Context<'_>) -> Poll<Instant> {
        Poll::Ready(self.tick_inner())
    }

    /// reset
    pub fn reset(&mut self) {
        self.start = None;
        self.ticks = 0;
    }
}

/// Timeout a future
pub fn tokio_timeout<F>(d: Duration, f: F) -> Timeout<F>
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
