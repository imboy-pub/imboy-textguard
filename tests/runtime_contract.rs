pub use imboy_textguard::model;
#[path = "../src/runtime/mod.rs"]
mod runtime;

use model::ErrorCode;
use runtime::{Admission, Transport, WAITING, WORKERS};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_retains_worker_until_actual_work_ends() {
    let gate = Admission::default();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let held = gate.clone();
    let request = tokio::spawn(async move {
        held.execute(Instant::now() + Duration::from_millis(25), move |_| {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            Ok(())
        })
        .await
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(request.await.unwrap(), Err(ErrorCode::CheckDeadline));
    assert_eq!(gate.counts().running, 1);
    assert_eq!(gate.counts().cancelled_still_running, 1);
    finish_tx.send(()).unwrap();
    for _ in 0..100 {
        if gate.counts().running == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(gate.counts().running, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_client_future_signals_cancel_without_freeing_worker() {
    let gate = Admission::default();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let held = gate.clone();
    let request = tokio::spawn(async move {
        held.execute(Instant::now() + Duration::from_secs(1), move |token| {
            started_tx.send(token).unwrap();
            finish_rx.recv().unwrap();
            Ok(())
        })
        .await
    });
    let token = started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    request.abort();
    let _ = request.await;
    assert!(token.is_cancelled());
    assert_eq!(gate.counts().running, 1);
    finish_tx.send(()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_callers_share_workers_and_bounded_admission() {
    let gate = Admission::default();
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut requests = Vec::new();
    for _ in 0..80 {
        let held = gate.clone();
        let active = current.clone();
        let peak = peak.clone();
        requests.push(tokio::spawn(async move {
            held.execute(Instant::now() + Duration::from_millis(80), move |_| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        }));
    }
    let mut rejected = 0;
    for request in requests {
        match request.await.unwrap() {
            Ok(()) => (),
            Err(ErrorCode::Overloaded) => rejected += 1,
            other => panic!("{other:?}"),
        }
    }
    assert!(rejected > 0);
    assert_eq!(peak.load(Ordering::SeqCst), WORKERS);
    assert_eq!(gate.counts().queued, 0);
    assert_eq!(gate.counts().running, 0);
    assert!(gate.counts().rejected > 0);
    assert_eq!(WAITING, 32);
}

#[tokio::test]
async fn expired_queue_work_never_starts_and_success_returns() {
    let gate = Admission::default();
    assert_eq!(
        gate.execute(Instant::now() - Duration::from_millis(1), |_| panic!(
            "expired job started"
        ))
        .await,
        Err::<(), _>(ErrorCode::CheckDeadline)
    );
    assert_eq!(
        gate.execute(Instant::now() + Duration::from_millis(80), |_| Ok(42))
            .await,
        Ok(42)
    );
}

#[test]
fn body_and_transport_limits_are_shared_and_reusable() {
    let gate = Admission::default();
    let readers: Vec<_> = (0..64).map(|_| gate.reader().unwrap()).collect();
    assert!(matches!(gate.clone().reader(), Err(ErrorCode::Overloaded)));
    drop(readers);
    assert!(gate.reader().is_ok());
    for transport in [Transport::Http, Transport::Grpc] {
        let mut connections = Vec::new();
        for _ in 0..128 {
            connections.push(gate.connection(transport).unwrap());
        }
        assert!(matches!(
            gate.connection(transport),
            Err(ErrorCode::Overloaded)
        ));
        drop(connections);
    }
}
