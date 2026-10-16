use crate::model::ErrorCode;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const WORKERS: usize = 2;
pub const WAITING: usize = 32;
pub const ADMISSION_WAIT: Duration = Duration::from_millis(5);
pub const CHECK_BUDGET: Duration = Duration::from_millis(80);

pub struct Cancellation {
    cancelled: AtomicBool,
    deadline: Instant,
}

impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
}

struct CancelOnDrop(Arc<Cancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }
}

struct Inner {
    workers: Arc<Semaphore>,
    waiting: Arc<Semaphore>,
    readers: Arc<Semaphore>,
    http: Arc<Semaphore>,
    grpc: Arc<Semaphore>,
    running: Mutex<Vec<Arc<Cancellation>>>,
    rejected: AtomicU64,
}

#[derive(Clone)]
pub struct Admission(Arc<Inner>);

#[derive(Debug, PartialEq, Eq)]
pub struct Counts {
    pub queued: usize,
    pub running: usize,
    pub cancelled_still_running: usize,
    pub rejected: u64,
}

#[derive(Clone, Copy)]
pub enum Transport {
    Http,
    Grpc,
}

struct Running {
    inner: Arc<Inner>,
    cancellation: Arc<Cancellation>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.inner
            .running
            .lock()
            .unwrap()
            .retain(|token| !Arc::ptr_eq(token, &self.cancellation));
    }
}

impl Default for Admission {
    fn default() -> Self {
        Self(Arc::new(Inner {
            workers: Arc::new(Semaphore::new(WORKERS)),
            waiting: Arc::new(Semaphore::new(WAITING)),
            readers: Arc::new(Semaphore::new(64)),
            http: Arc::new(Semaphore::new(128)),
            grpc: Arc::new(Semaphore::new(128)),
            running: Mutex::new(Vec::new()),
            rejected: AtomicU64::new(0),
        }))
    }
}

impl Admission {
    fn acquire(&self, semaphore: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit, ErrorCode> {
        semaphore.clone().try_acquire_owned().map_err(|_| {
            self.0.rejected.fetch_add(1, Ordering::Relaxed);
            ErrorCode::Overloaded
        })
    }

    pub fn reader(&self) -> Result<OwnedSemaphorePermit, ErrorCode> {
        self.acquire(&self.0.readers)
    }

    pub fn connection(&self, transport: Transport) -> Result<OwnedSemaphorePermit, ErrorCode> {
        self.acquire(match transport {
            Transport::Http => &self.0.http,
            Transport::Grpc => &self.0.grpc,
        })
    }

    pub fn counts(&self) -> Counts {
        let running = self.0.running.lock().unwrap();
        Counts {
            queued: WAITING - self.0.waiting.available_permits(),
            running: running.len(),
            cancelled_still_running: running.iter().filter(|token| token.is_cancelled()).count(),
            rejected: self.0.rejected.load(Ordering::Relaxed),
        }
    }

    async fn worker(&self, deadline: Instant) -> Result<OwnedSemaphorePermit, ErrorCode> {
        if Instant::now() >= deadline {
            return Err(ErrorCode::CheckDeadline);
        }
        let waiting = self.acquire(&self.0.waiting)?;
        let admission_deadline = deadline.min(Instant::now() + ADMISSION_WAIT);
        let acquired = tokio::time::timeout_at(
            admission_deadline.into(),
            self.0.workers.clone().acquire_owned(),
        )
        .await;
        let permit = match acquired {
            Ok(Ok(permit)) => permit,
            _ => {
                self.0.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(if Instant::now() >= deadline {
                    ErrorCode::CheckDeadline
                } else {
                    ErrorCode::Overloaded
                });
            }
        };
        drop(waiting);
        if Instant::now() >= deadline {
            return Err(ErrorCode::CheckDeadline);
        }
        Ok(permit)
    }

    pub async fn execute<Output, Work>(
        &self,
        deadline: Instant,
        work: Work,
    ) -> Result<Output, ErrorCode>
    where
        Output: Send + 'static,
        Work: FnOnce(Arc<Cancellation>) -> Result<Output, ErrorCode> + Send + 'static,
    {
        let deadline = deadline.min(Instant::now() + CHECK_BUDGET);
        let permit = self.worker(deadline).await?;
        let cancellation = Arc::new(Cancellation {
            cancelled: AtomicBool::new(false),
            deadline,
        });
        let _cancel = CancelOnDrop(cancellation.clone());
        self.0.running.lock().unwrap().push(cancellation.clone());
        let running = Running {
            inner: self.0.clone(),
            cancellation: cancellation.clone(),
            _permit: permit,
        };
        let task = tokio::task::spawn_blocking(move || {
            let _running = running;
            if cancellation.is_cancelled() {
                return Err(ErrorCode::CheckDeadline);
            }
            let result = work(cancellation.clone());
            if cancellation.is_cancelled() {
                Err(ErrorCode::CheckDeadline)
            } else {
                result
            }
        });
        match tokio::time::timeout_at(deadline.into(), task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ErrorCode::InternalError),
            Err(_) => Err(ErrorCode::CheckDeadline),
        }
    }
}
