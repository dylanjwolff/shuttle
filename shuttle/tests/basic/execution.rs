use shuttle::{
    check, check_dfs, current,
    scheduler::{DfsScheduler, RandomScheduler},
    thread, Config, MaxSteps, Runner,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
// Not actually trying to explore interleavings involving AtomicUsize, just using to smuggle a
// mutable counter across threads
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use test_log::test;

#[test]
fn basic_scheduler_test() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    check(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let counter_clone = Arc::clone(&counter);
        thread::spawn(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });
        counter.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(counter_clone.load(Ordering::SeqCst), 3);
}

#[test]
fn max_steps_none() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let mut config = Config::new();
    config.max_steps = MaxSteps::None;

    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new(scheduler, config);
    runner.run(move || {
        for _ in 0..100 {
            counter.fetch_add(1, Ordering::SeqCst);
            thread::yield_now();
        }
    });

    assert_eq!(counter_clone.load(Ordering::SeqCst), 100 * 10);
}

#[test]
fn max_steps_continue() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let mut config = Config::new();
    config.max_steps = MaxSteps::ContinueAfter(50);

    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new(scheduler, config);
    runner.run(move || {
        for _ in 0..100 {
            counter.fetch_add(1, Ordering::SeqCst);
            thread::yield_now();
        }
    });

    assert_eq!(counter_clone.load(Ordering::SeqCst), 50 * 10);
}

#[test]
fn max_steps_fail() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let mut config = Config::new();
    config.max_steps = MaxSteps::FailAfter(50);

    let scheduler = RandomScheduler::new(10);
    let runner = Runner::new(scheduler, config);
    let result = catch_unwind(AssertUnwindSafe(move || {
        runner.run(move || {
            for _ in 0..100 {
                counter.fetch_add(1, Ordering::SeqCst);
                thread::yield_now();
            }
        })
    }));

    assert!(result.is_err());
    assert_eq!(counter_clone.load(Ordering::SeqCst), 50);
}

// Test that a scheduler can return `None` to trigger the same behavior as `MaxSteps::ContinueAfter`
#[test]
fn max_steps_early_exit_scheduler() {
    use shuttle::scheduler::{Schedule, Scheduler, Task, TaskId};

    #[derive(Debug)]
    struct EarlyExitScheduler {
        iterations: usize,
        max_iterations: usize,
        steps: usize,
        max_steps: usize,
    }

    impl EarlyExitScheduler {
        fn new(max_iterations: usize, max_steps: usize) -> Self {
            Self {
                iterations: 0,
                max_iterations,
                steps: 0,
                max_steps,
            }
        }
    }

    impl Scheduler for EarlyExitScheduler {
        fn new_execution(&mut self) -> Option<Schedule> {
            if self.iterations >= self.max_iterations {
                None
            } else {
                self.iterations += 1;
                self.steps = 0;
                Some(Schedule::new(0))
            }
        }

        fn next_task(
            &mut self,
            runnable_tasks: &[&Task],
            _current_task: Option<TaskId>,
            _is_yielding: bool,
        ) -> Option<TaskId> {
            if self.steps >= self.max_steps {
                None
            } else {
                self.steps += 1;
                Some(runnable_tasks.first().unwrap().id())
            }
        }

        fn next_u64(&mut self) -> u64 {
            unimplemented!()
        }
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let mut config = Config::new();
    config.max_steps = MaxSteps::FailAfter(51);

    let scheduler = EarlyExitScheduler::new(10, 50);
    let runner = Runner::new(scheduler, config);
    runner.run(move || {
        for _ in 0..100 {
            counter.fetch_add(1, Ordering::SeqCst);
            thread::yield_now();
        }
    });

    assert_eq!(counter_clone.load(Ordering::SeqCst), 50 * 10);
}

#[test]
#[should_panic]
fn context_switches_outside_execution() {
    current::context_switches();
}

#[test]
fn context_switches_atomic() {
    // The current implementation makes the following context switches:
    // 1 initial
    // 2 spawns
    // 2 joins
    // 2 thread terminations
    // 4 `fetch_add` (one before and one after each)
    const EXPECTED_CONTEXT_SWITCHES: usize = 11;

    check_dfs(
        move || {
            let mut threads = vec![];
            let counter = Arc::new(shuttle::sync::atomic::AtomicUsize::new(0));

            assert_eq!(current::context_switches(), 1);

            for _ in 0..2 {
                let counter = Arc::clone(&counter);

                threads.push(thread::spawn(move || {
                    let count = counter.fetch_add(1, Ordering::SeqCst) + 1;

                    // We saw the initial context switch, the spawn and first context switch for each `fetch_add`,
                    // and the second context switch after the `fetch_add` of this thread.
                    assert!(current::context_switches() >= 2 + 2 * count);

                    // We did not see the last context switch of this thread.
                    assert!(current::context_switches() < EXPECTED_CONTEXT_SWITCHES);
                }));
            }

            for thread in threads {
                thread.join().unwrap();
            }

            assert_eq!(current::context_switches(), EXPECTED_CONTEXT_SWITCHES);
        },
        None,
    );
}

#[test]
fn context_switches_mutex() {
    use shuttle::sync::Mutex;

    check_dfs(
        move || {
            let mutex1 = Arc::new(Mutex::new(0));
            let mutex2 = Arc::new(Mutex::new(0));

            assert_eq!(current::context_switches(), 1);

            {
                let mutex1 = mutex1.lock().unwrap();
                assert_eq!(current::context_switches(), 2);
                {
                    let mutex2 = mutex2.lock().unwrap();
                    assert_eq!(current::context_switches(), 3);
                    drop(mutex2);
                }
                assert_eq!(current::context_switches(), 4);
                drop(mutex1);
            }

            assert_eq!(current::context_switches(), 5);
        },
        None,
    );
}

/// Check that we get a good failure message if accessing a Shuttle primitive from outside an
/// execution.
#[test]
#[should_panic(expected = "are you trying to access a Shuttle primitive from outside a Shuttle test?")]
fn failure_outside_execution() {
    let lock = shuttle::sync::Mutex::new(0u64);
    let _ = lock.lock().unwrap();
}

fn reset_step_count(do_reset: bool, step_bound: usize) {
    let mut config = Config::new();
    config.max_steps = MaxSteps::FailAfter(step_bound);

    let scheduler = DfsScheduler::new(None, false);
    let runner = Runner::new(scheduler, config);

    runner.run(move || {
        (0..4)
            .map(move |_| {
                thread::spawn(move || {
                    for _ in 0..3 {
                        thread::yield_now();
                    }
                    if do_reset {
                        shuttle::current::reset_step_count();
                    }
                })
            })
            .for_each(|jh| jh.join().unwrap())
    });
}

#[test]
#[should_panic(expected = "exceeded max_steps bound")]
fn dont_reset_step_count() {
    reset_step_count(false, 20);
}

#[test]
fn do_reset_step_count() {
    reset_step_count(true, 7);
}

#[test]
#[should_panic(expected = "exceeded max_steps bound")]
fn do_reset_step_count_panics() {
    reset_step_count(true, 6);
}

// Common test infrastructure for task signature tests
mod task_signature_test {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{trace, Event, Id, Metadata, Subscriber};

    #[derive(Clone)]
    pub struct SignatureSubscriber {
        pub signatures: Arc<Mutex<HashMap<u64, usize>>>,
        pub static_create_locations: Arc<Mutex<HashMap<u64, usize>>>,
    }

    impl SignatureSubscriber {
        pub fn new() -> Self {
            Self {
                signatures: Arc::new(Mutex::new(HashMap::new())),
                static_create_locations: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    impl Subscriber for SignatureSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let metadata = event.metadata();
            if metadata.target() == "shuttle::runtime::task" && metadata.level() == &tracing::Level::INFO {
                struct SignatureVisitor {
                    task_id: Option<String>,
                    signature: Option<u64>,
                    static_create_location: Option<u64>,
                }
                impl Visit for SignatureVisitor {
                    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                        if field.name() == "task_id" {
                            self.task_id = Some(format!("{:?}", value));
                        }
                    }
                    fn record_u64(&mut self, field: &Field, value: u64) {
                        if field.name() == "signature" {
                            self.signature = Some(value);
                        }
                        if field.name() == "static_create_location" {
                            self.static_create_location = Some(value);
                        }
                    }
                }
                let mut visitor = SignatureVisitor {
                    task_id: None,
                    signature: None,
                    static_create_location: None,
                };
                event.record(&mut visitor);

                if let Some(sig) = visitor.signature {
                    self.signatures
                        .lock()
                        .unwrap()
                        .entry(sig)
                        .and_modify(|counter| *counter += 1)
                        .or_insert(1);
                }
                if let Some(loc) = visitor.static_create_location {
                    self.static_create_locations
                        .lock()
                        .unwrap()
                        .entry(loc)
                        .and_modify(|counter| *counter += 1)
                        .or_insert(1);
                }
            }
        }

        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    pub fn check_n_same_signatures(signatures: &Arc<Mutex<HashMap<u64, usize>>>, expected_count: usize) {
        let signatures = signatures.lock().unwrap();
        trace!("Total signatures captured: {}", signatures.len());

        trace!("Signature counts: {:?}", signatures);

        let worker_signatures: Vec<u64> = signatures
            .iter()
            .filter(|(_, count)| **count == expected_count)
            .map(|(sig, _)| *sig)
            .collect();

        assert_eq!(
            worker_signatures.len(),
            1,
            "Should have exactly one signature appearing {} times",
            expected_count
        );
        trace!(
            "{} tasks have the same signature: {}",
            expected_count, worker_signatures[0]
        );
    }

    pub fn check_n_different_signatures(signatures: &Arc<Mutex<HashMap<u64, usize>>>, expected_count: usize) {
        let signatures = signatures.lock().unwrap();
        trace!("Total signatures captured: {}", signatures.len());

        trace!("Signature counts: {:?}", signatures);

        let unique_signatures: Vec<u64> = signatures.keys().cloned().collect();
        assert_eq!(
            unique_signatures.len(),
            expected_count,
            "Should have {} different signatures",
            expected_count
        );
        trace!(
            "All {} tasks have different signatures: {:?}",
            expected_count, unique_signatures
        );
    }

    pub fn run_test_n_iterations_with_subscriber<F>(
        test_fn: F,
        iterations: usize,
    ) -> (Arc<Mutex<HashMap<u64, usize>>>, Arc<Mutex<HashMap<u64, usize>>>)
    where
        F: Fn() + Send + Sync + 'static,
    {
        use shuttle::{scheduler::RandomScheduler, Runner};

        let subscriber = SignatureSubscriber::new();
        let signatures = Arc::clone(&subscriber.signatures);
        let static_create_locations = Arc::clone(&subscriber.static_create_locations);
        let _guard = tracing::subscriber::set_default(subscriber);

        let scheduler = RandomScheduler::new(iterations);
        let runner = Runner::new(scheduler, Default::default());
        runner.run(test_fn);

        (signatures, static_create_locations)
    }
}

#[test]
fn task_signatures_same_function() {
    use task_signature_test::{
        check_n_different_signatures, check_n_same_signatures, run_test_n_iterations_with_subscriber,
    };

    fn worker_function() {}

    let (signatures, static_create_locations) = run_test_n_iterations_with_subscriber(
        || {
            let mut handles = Vec::new();
            for _ in 0..10 {
                handles.push(thread::spawn(worker_function));
            }
            for handle in handles {
                handle.join().unwrap();
            }
        },
        1,
    );

    check_n_same_signatures(&static_create_locations, 10);
    check_n_different_signatures(&signatures, 11);
}

#[test]
fn task_signatures_same_function_async() {
    use shuttle::future;
    use task_signature_test::{
        check_n_different_signatures, check_n_same_signatures, run_test_n_iterations_with_subscriber,
    };

    async fn async_worker_function() {}

    let (signatures, static_create_locations) = run_test_n_iterations_with_subscriber(
        || {
            let mut handles = Vec::new();
            for _ in 0..10 {
                handles.push(future::spawn(async_worker_function()));
            }
            future::block_on(async {
                for handle in handles {
                    handle.await.unwrap();
                }
            });
        },
        1,
    );

    check_n_same_signatures(&static_create_locations, 10);
    check_n_different_signatures(&signatures, 11);
}

#[test]
fn task_signatures_different_functions() {
    use task_signature_test::{check_n_different_signatures, run_test_n_iterations_with_subscriber};

    fn worker_function_1() {}
    fn worker_function_2() {}

    let (signatures, _) = run_test_n_iterations_with_subscriber(
        || {
            let handle1 = thread::spawn(worker_function_1);
            let handle2 = thread::spawn(worker_function_2);
            handle1.join().unwrap();
            handle2.join().unwrap();
        },
        1,
    );

    check_n_different_signatures(&signatures, 3);
}

#[test]
fn task_signatures_different_functions_async() {
    use shuttle::future;
    use task_signature_test::{check_n_different_signatures, run_test_n_iterations_with_subscriber};

    async fn async_worker_function_1() {}
    async fn async_worker_function_2() {}

    let (signatures, _) = run_test_n_iterations_with_subscriber(
        || {
            let handle1 = future::spawn(async_worker_function_1());
            let handle2 = future::spawn(async_worker_function_2());
            future::block_on(async {
                handle1.await.unwrap();
                handle2.await.unwrap();
            });
        },
        1,
    );

    check_n_different_signatures(&signatures, 3);
}
#[test]
fn task_signatures_consistent_across_iterations() {
    use task_signature_test::{check_n_different_signatures, run_test_n_iterations_with_subscriber};

    fn worker_with_nested_spawn() {
        let handle = thread::spawn(|| {});
        handle.join().unwrap();
    }

    let (signatures, _) = run_test_n_iterations_with_subscriber(
        || {
            let handle1 = thread::spawn(worker_with_nested_spawn);
            let handle2 = thread::spawn(worker_with_nested_spawn);
            handle1.join().unwrap();
            handle2.join().unwrap();
        },
        100,
    );

    check_n_different_signatures(&signatures, 5);
}
