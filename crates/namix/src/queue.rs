//! In-process queue and asynchronous job contract.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result as AnyResult, anyhow};
use tokio::sync::{Mutex, mpsc};

/// Jobs are infrastructure boundaries: arbitrary driver failures retain their
/// error chain and optional `anyhow::Context` until the worker records them.
pub type JobResult = AnyResult<()>;
pub type JobFuture = Pin<Box<dyn Future<Output = JobResult> + Send>>;

pub trait Job: Send + 'static {
    fn name(&self) -> &'static str;
    fn handle(self: Box<Self>) -> JobFuture;
}

#[derive(Clone)]
pub struct Queue {
    tx: mpsc::Sender<Box<dyn Job>>,
    rx: Arc<Mutex<mpsc::Receiver<Box<dyn Job>>>>,
}

impl Queue {
    pub fn memory(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub async fn dispatch(&self, job: impl Job) -> JobResult {
        self.tx
            .send(Box::new(job))
            .await
            .map_err(|_| anyhow!("queue is closed"))
    }

    pub async fn work_once(&self) -> Option<(&'static str, JobResult)> {
        let job = self.rx.lock().await.recv().await?;
        let name = job.name();
        Some((name, job.handle().await))
    }

    pub fn worker(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some((name, result)) = self.work_once().await {
                if let Err(error) = result {
                    tracing::error!(job = name, error = ?error, "queued job failed");
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Hit(Arc<tokio::sync::Mutex<bool>>);

    impl Job for Hit {
        fn name(&self) -> &'static str {
            "hit"
        }

        fn handle(self: Box<Self>) -> JobFuture {
            Box::pin(async move {
                *self.0.lock().await = true;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn dispatches_memory_job() {
        let queue = Queue::memory(1);
        let hit = Arc::new(tokio::sync::Mutex::new(false));
        queue.dispatch(Hit(hit.clone())).await.unwrap();
        assert_eq!(queue.work_once().await.unwrap().0, "hit");
        assert!(*hit.lock().await);
    }

    struct Fails;

    impl Job for Fails {
        fn name(&self) -> &'static str {
            "fails"
        }

        fn handle(self: Box<Self>) -> JobFuture {
            Box::pin(async { Err(anyhow::anyhow!("smtp unavailable").context("welcome mail")) })
        }
    }

    #[tokio::test]
    async fn preserves_anyhow_job_context() {
        let queue = Queue::memory(1);
        queue.dispatch(Fails).await.unwrap();
        let (_, error) = queue.work_once().await.unwrap();
        assert!(format!("{:#}", error.unwrap_err()).contains("welcome mail: smtp unavailable"));
    }
}
