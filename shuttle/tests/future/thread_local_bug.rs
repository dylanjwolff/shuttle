use shuttle::{check_random, future, thread_local};
use std::cell::RefCell;
use test_log::test;
use tracing::info;

thread_local! {
    static COUNTER: RefCell<u32> = RefCell::new(0);
}

#[test]
fn async_thread_local_shared_bug() {
    // This test demonstrates that futures should share thread-local storage
    // when running on the same OS thread, but Shuttle may give them separate storage
    check_random(
        || {
            let future1 = async {
                COUNTER.with(|c| {
                    // Set TLS to be 42
                    *c.borrow_mut() = 42;
                });
                info!("Future 1 done, TLS has been set");
                future::yield_now().await;
            };

            let future2 = async {
                let mut val = 41;
                for _i in 0..1000 {
                    // do many yields to ensure that future1 is certain (w/ high-probability) to complete before the assertion in future2 is reached
                    future::yield_now().await;
                    val = COUNTER.with(|c| *c.borrow());
                }
                info!("Future 2 done, asserting TLS value observed");
                assert_eq!(
                    val, 42,
                    "Future 2 should see value set by Future 1 since they are on the same OS thread"
                );
            };

            future::block_on(async {
                let handle1 = future::spawn(future1);
                let handle2 = future::spawn(future2);

                handle1.await.unwrap();
                handle2.await.unwrap();
            });
        },
        100,
    );
}
