use crate::runtime::{execution::ExecutionState, task::TaskId};
use std::cmp::{Ordering, PartialOrd};

use crate::runtime::task::DEFAULT_INLINE_TASKS;
use smallvec::{smallvec, SmallVec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorClock {
    time: Option<SmallVec<[u32; DEFAULT_INLINE_TASKS]>>,
}

impl VectorClock {
    pub(crate) fn new() -> Self {
        Self::new_enabled(ExecutionState::use_vector_clocks())
    }
    pub(crate) const fn const_new() -> Self {
        Self {
            time: Some(SmallVec::new_const()),
        }
    }

    pub(crate) fn new_enabled(enabled: bool) -> Self {
        Self {
            time: if enabled { Some(SmallVec::new()) } else { None },
        }
    }

    // Zero extend clock to accommodate `task_id` tasks.
    pub(crate) fn extend(&mut self, task_id: TaskId) {
        if let Some(ref mut time) = self.time {
            let num_new_tasks = 1 + task_id.0 - time.len();
            let clock: SmallVec<[_; DEFAULT_INLINE_TASKS]> = smallvec![0u32; num_new_tasks];
            time.extend_from_slice(&clock);
        }
    }

    pub(crate) fn increment(&mut self, task_id: TaskId) {
        if let Some(ref mut time) = self.time {
            time[task_id.0] += 1;
        }
    }

    // Update the clock of `self` with the clock from `other`
    pub(crate) fn update(&mut self, other: &Self) {
        if let (Some(ref mut self_time), Some(ref other_time)) = (&mut self.time, &other.time) {
            let n1 = self_time.len();
            let n2 = other_time.len();
            for i in 0..n1.min(n2) {
                self_time[i] = self_time[i].max(other_time[i])
            }
            for i in n1..n2 {
                self_time.push(other_time[i]);
            }
        }
    }

    pub fn get(&self, i: usize) -> u32 {
        self.time.as_ref().map(|time| time[i]).unwrap_or(0)
    }
}

impl<const N: usize> From<&[u32; N]> for VectorClock {
    fn from(v: &[u32; N]) -> Self {
        Self {
            time: Some(SmallVec::from(&v[..])),
        }
    }
}

impl From<&[u32]> for VectorClock {
    fn from(v: &[u32]) -> Self {
        Self {
            time: Some(SmallVec::from(v)),
        }
    }
}

impl std::ops::Deref for VectorClock {
    type Target = [u32];
    fn deref(&self) -> &Self::Target {
        self.time.as_ref().map(|time| &time[..]).unwrap_or(&[])
    }
}

fn unify(a: Ordering, b: Ordering) -> Option<Ordering> {
    use Ordering::*;

    match (a, b) {
        (Equal, Equal) => Some(Equal),
        (Less, Greater) | (Greater, Less) => None,
        (Less, _) | (_, Less) => Some(Less),
        (Greater, _) | (_, Greater) => Some(Greater),
    }
}

impl PartialOrd for VectorClock {
    // Compare vector clocks
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (&self.time, &other.time) {
            (Some(self_time), Some(other_time)) => {
                let n1 = self_time.len();
                let n2 = other_time.len();
                let mut ord = n1.cmp(&n2);
                for i in 0..n1.min(n2) {
                    ord = unify(ord, self_time[i].cmp(&other_time[i]))?;
                }
                Some(ord)
            }
            _ => Some(Ordering::Equal),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn vector_clock() {
        let v1 = VectorClock::from(&[1, 2, 3, 4]);
        let v2 = VectorClock::from(&[1, 2, 4, 5]);
        let v3 = VectorClock::from(&[1, 2, 3, 1]);
        let v4 = VectorClock::from(&[1, 2, 4, 1]);
        let v5 = VectorClock::from(&[1, 2, 3, 4]);
        assert!(v1 < v2 && v1 > v3 && v1 == v5);
        assert!(v2 > v3 && v2 > v4);
        assert!(v3 < v4);
        assert_eq!(v1.partial_cmp(&v4), None);

        let v1 = VectorClock::from(&[1, 2, 3, 4]);
        let v2 = VectorClock::from(&[1, 2, 2]);
        let v3 = VectorClock::from(&[1, 2, 3]);
        let v4 = VectorClock::from(&[1, 2, 4]);
        assert!(v1 > v2);
        assert!(v1 > v3);
        assert_eq!(v1.partial_cmp(&v4), None);

        let v1 = VectorClock::from(&[]);
        let v2 = VectorClock::from(&[1]);
        assert!(v1 < v2);

        let v1 = VectorClock::from(&[1, 2, 1]);
        let v2 = VectorClock::from(&[1, 3]);
        let v3 = VectorClock::from(&[1, 1, 1, 2]);
        let v4 = VectorClock::from(&[1, 1, 2]);

        let mut v = v1.clone();
        v.update(&v2);
        assert_eq!(v, VectorClock::from(&[1, 3, 1]));

        let mut v = v1.clone();
        v.update(&v3);
        assert_eq!(v, VectorClock::from(&[1, 2, 1, 2]));

        let mut v = v1.clone();
        v.update(&v4);
        assert_eq!(v, VectorClock::from(&[1, 2, 2]));

        let mut v = v1.clone();
        v.update(&VectorClock::new());
        assert_eq!(v, v1);
    }
}
