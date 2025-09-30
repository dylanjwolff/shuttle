use shuttle::current::{me, set_label_for_task};
use shuttle::scheduler::{DfsScheduler, RandomScheduler};
use shuttle::sync::time::constant_stepped::{ConstantSteppedTimeModel, ConstantTimeDistribution};
use shuttle::sync::time::frozen::FrozenTimeModel;
use shuttle::sync::time::{async_interval, async_sleep, async_timeout, trigger_timeouts, Duration, Instant};
use shuttle::{future, thread, Config, Runner};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tracing::trace;

#[test]
fn test_stepped_blocking_sleep() {
    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());
    runner.run(|| {
        let start = Instant::now();
        thread::sleep(Duration::from_millis(50) + Duration::from_millis(50));
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() == 100);
    });
}

/// This test includes 3 scheduling points which should advance the global time:
/// - 1 spawn and 1 yield on the main thread
/// - 1 yield on the child thread
///
/// Thus with a stepped time, it should be possible for either side of the condition to be met:
/// - True => both threads have executed (3 events * time_step)
/// - False => only the main thread has executed (2 events * time_step)
#[test]
fn test_stepped_elapsed_time() {
    let less_than_count = Arc::new(Mutex::new(0));
    let greater_than_count = Arc::new(Mutex::new(0));

    let time_step = Duration::from_micros(10);

    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = DfsScheduler::new(None, false);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());

    let less_count_inner = less_than_count.clone();
    let greater_count_inner = greater_than_count.clone();

    runner.run(move || {
        let start = Instant::now();
        thread::spawn(|| {
            thread::yield_now();
        });
        thread::yield_now();
        let elapsed = start.elapsed();
        trace!("elapsed {:?}", elapsed);

        if elapsed > 2 * time_step {
            *greater_count_inner.lock().unwrap() += 1;
        } else {
            *less_count_inner.lock().unwrap() += 1;
        }
    });

    let less_count = *less_than_count.lock().unwrap();
    let greater_count = *greater_than_count.lock().unwrap();

    assert!(
        less_count > 0,
        "Expected some executions with elapsed <= {:?}, got {}",
        time_step * 2,
        less_count
    );
    assert!(
        greater_count > 0,
        "Expected some executions with elapsed > {:?}, got {}",
        time_step * 2,
        greater_count
    );
}

#[test]
fn test_stepped_async_sleep() {
    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());
    runner.run(|| {
        future::block_on(async {
            let start = Instant::now();
            async_sleep(Duration::from_millis(50)).await;
            let elapsed = start.elapsed();
            assert_eq!(elapsed.as_millis(), 50);
        });
    });
}

#[test]
fn test_stepped_timeout_expired() {
    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());
    runner.run(|| {
        future::block_on(async {
            let start = Instant::now();
            let result = async_timeout(Duration::from_millis(50), async {
                async_sleep(Duration::from_millis(100)).await;
                42
            })
            .await;
            trace!("elapsed time according to model {:?}", start);
            assert!(start.elapsed() < Duration::from_millis(100));
            assert!(result.is_err());
        });
    });
}

#[test]
fn test_stepped_timeout_not_expired() {
    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());
    runner.run(|| {
        future::block_on(async {
            let result = async_timeout(Duration::from_millis(50), async {
                async_sleep(Duration::from_millis(20)).await;
                42
            })
            .await;
            assert_eq!(result.unwrap(), 42);
        });
    });
}

#[test]
fn test_async_interval() {
    let time_model = ConstantSteppedTimeModel::new(ConstantTimeDistribution::new(std::time::Duration::from_micros(10)));
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());
    runner.run(|| {
        future::block_on(async {
            let mut interval = async_interval(Duration::from_millis(10));
            let start = Instant::now();
            interval.tick().await;
            let first_tick = start.elapsed();
            interval.tick().await;
            let second_tick = start.elapsed();
            interval.tick().await;
            let third_tick = start.elapsed();

            assert_eq!(first_tick.as_millis(), 0);
            assert_eq!(second_tick.as_millis(), 10);
            assert_eq!(third_tick.as_millis(), 20);
        });
    });
}

#[derive(Clone, Debug, PartialEq)]
struct TaskType(String);

/// Probabilistically force a switch for random schedulers [P(no switch) = 1/T^bound for T threads]
/// Returns true if the condition ever holds while spinning.
fn spin_switch_and_get_any<F>(condition: F) -> bool
where
    F: Fn() -> bool,
{
    let bound = 10000;
    let mut ret = false;
    for _ in 0..bound {
        thread::yield_now();
        ret |= condition();
    }
    ret
}

#[test]
fn test_frozen_trigger_timeouts() {
    let time_model = FrozenTimeModel::new();
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());

    runner.run(|| {
        let woken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let woken_clone = woken.clone();

        let handle = thread::spawn(move || {
            set_label_for_task(me(), TaskType("target".to_string()));

            future::block_on(async {
                async_sleep(Duration::from_millis(100)).await;
                woken_clone.store(true, Ordering::SeqCst);
            });
        });

        // Verify the task has not completed yet because it is asleep
        assert!(!spin_switch_and_get_any(|| woken.load(Ordering::SeqCst)));

        // Trigger timeouts for tasks with "target" label
        trigger_timeouts(|labels| labels.get::<TaskType>().map_or(false, |t| t.0 == "target"));

        // Verify the task completed after trigger_timeouts
        assert!(spin_switch_and_get_any(|| woken.load(Ordering::SeqCst)));

        // Wait for the task to complete
        handle.join().unwrap();
    });
}

#[test]
fn test_frozen_trigger_timeouts_async_timeout() {
    let time_model = FrozenTimeModel::new();
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());

    runner.run(|| {
        let timeout_triggered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timeout_triggered_clone = timeout_triggered.clone();

        let handle = thread::spawn(move || {
            set_label_for_task(me(), TaskType("target".to_string()));

            future::block_on(async {
                let result = async_timeout(Duration::from_millis(100), async {
                    async_sleep(Duration::from_millis(200)).await;
                    42
                })
                .await;

                timeout_triggered_clone.store(result.is_err(), Ordering::SeqCst);
            });
        });

        // Verify the timeout has not triggered yet
        assert!(!spin_switch_and_get_any(|| timeout_triggered.load(Ordering::SeqCst)));

        // Trigger timeouts for tasks with "target" label
        trigger_timeouts(|labels| labels.get::<TaskType>().map_or(false, |t| t.0 == "target"));

        // Verify the timeout was triggered
        assert!(spin_switch_and_get_any(|| timeout_triggered.load(Ordering::SeqCst)));

        // Wait for the task to complete
        handle.join().unwrap();
    });
}

#[test]
fn test_frozen_trigger_timeouts_selective() {
    let time_model = FrozenTimeModel::new();
    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new_with_time_model(scheduler, time_model, Config::new());

    runner.run(|| {
        let other_woken = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let other_woken_clone = other_woken.clone();

        let other_handle = thread::spawn(move || {
            set_label_for_task(me(), TaskType("other".to_string()));
            future::block_on(async {
                async_sleep(Duration::from_millis(100)).await;
                other_woken_clone.store(true, Ordering::SeqCst);
            });
        });

        // Verify neither task has completed yet
        assert!(!spin_switch_and_get_any(|| other_woken.load(Ordering::SeqCst)));

        // Trigger timeouts only for tasks with "target" label
        trigger_timeouts(|labels| labels.get::<TaskType>().map_or(false, |t| t.0 == "target"));

        // Verify other task was not triggered
        assert!(!spin_switch_and_get_any(|| other_woken.load(Ordering::SeqCst)));

        other_handle.join().unwrap();
    });
}
