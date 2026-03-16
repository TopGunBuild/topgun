//! Background worker for periodic and on-demand tasks.
//!
//! Provides a generic `BackgroundWorker<R>` that processes tasks from an mpsc channel
//! via a `BackgroundRunnable` implementation, with optional periodic tick callbacks.

use async_trait::async_trait;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// BackgroundRunnable trait
// ---------------------------------------------------------------------------

/// Trait for task handlers executed by `BackgroundWorker`.
///
/// Implementors define how individual tasks are processed, what happens on each
/// periodic tick, and how to clean up on shutdown.
#[async_trait]
pub trait BackgroundRunnable: Send + 'static {
    /// The type of task this runnable processes.
    type Task: Send + 'static;

    /// Process a single task.
    async fn run(&mut self, task: Self::Task);

    /// Called periodically (on each tick interval). Default is a no-op.
    async fn on_tick(&mut self) {}

    /// Called once when the worker is shutting down. Default is a no-op.
    async fn shutdown(&mut self) {}
}

// ---------------------------------------------------------------------------
// GcTask
// ---------------------------------------------------------------------------

/// Task variants for the garbage collection worker.
#[derive(Debug)]
pub enum GcTask {
    /// Run a full garbage collection sweep.
    RunFull,
    /// Run GC for a specific map.
    RunMap { map_name: String },
}

// ---------------------------------------------------------------------------
// BackgroundWorker
// ---------------------------------------------------------------------------

/// Generic background worker that processes tasks via an mpsc channel.
///
/// The worker spawns a tokio task that:
/// 1. Listens for tasks on the mpsc channel
/// 2. Calls `BackgroundRunnable::run()` for each task
/// 3. Periodically calls `BackgroundRunnable::on_tick()` at the configured interval
/// 4. Calls `BackgroundRunnable::shutdown()` when stopped
pub struct BackgroundWorker<R: BackgroundRunnable> {
    tx: Option<mpsc::Sender<R::Task>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl<R: BackgroundRunnable> BackgroundWorker<R> {
    /// Start the background worker with the given runnable and tick interval.
    ///
    /// Returns a `BackgroundWorker` handle that can be used to submit tasks
    /// and stop the worker. The channel capacity is fixed at 256.
    pub fn start(mut runnable: R, tick_interval_ms: u64) -> Self {
        let (tx, mut rx) = mpsc::channel::<R::Task>(256);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            let mut tick_interval =
                tokio::time::interval(std::time::Duration::from_millis(tick_interval_ms));
            // Skip the first immediate tick so on_tick doesn't fire at startup.
            tick_interval.tick().await;

            loop {
                tokio::select! {
                    task = rx.recv() => {
                        match task {
                            Some(t) => runnable.run(t).await,
                            None => break, // Channel closed.
                        }
                    }
                    _ = tick_interval.tick() => {
                        runnable.on_tick().await;
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }

            runnable.shutdown().await;
        });

        Self {
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    /// Submit a task to the worker.
    ///
    /// # Errors
    ///
    /// Returns an error if the worker has been stopped or the channel is full.
    pub async fn submit(&self, task: R::Task) -> anyhow::Result<()> {
        match &self.tx {
            Some(tx) => tx
                .send(task)
                .await
                .map_err(|_| anyhow::anyhow!("worker channel closed")),
            None => Err(anyhow::anyhow!("worker not running")),
        }
    }

    /// Stop the worker gracefully, waiting for the worker task to complete.
    pub async fn stop(&mut self) {
        // Signal shutdown.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Close the task channel.
        self.tx.take();
        // Wait for the worker task to finish.
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;

    struct CountingRunnable {
        run_count: Arc<AtomicU32>,
        tick_count: Arc<AtomicU32>,
        shutdown_called: Arc<AtomicU32>,
    }

    #[async_trait]
    impl BackgroundRunnable for CountingRunnable {
        type Task = String;

        async fn run(&mut self, _task: String) {
            self.run_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_tick(&mut self) {
            self.tick_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn shutdown(&mut self) {
            self.shutdown_called.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn start_submit_and_stop() {
        let run_count = Arc::new(AtomicU32::new(0));
        let tick_count = Arc::new(AtomicU32::new(0));
        let shutdown_called = Arc::new(AtomicU32::new(0));

        let runnable = CountingRunnable {
            run_count: run_count.clone(),
            tick_count: tick_count.clone(),
            shutdown_called: shutdown_called.clone(),
        };

        let mut worker = BackgroundWorker::start(runnable, 60_000);

        worker.submit("task-1".to_string()).await.unwrap();
        worker.submit("task-2".to_string()).await.unwrap();
        worker.submit("task-3".to_string()).await.unwrap();

        // Give the worker time to process tasks.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(run_count.load(Ordering::SeqCst), 3);

        worker.stop().await;

        assert_eq!(shutdown_called.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tick_fires_periodically() {
        let run_count = Arc::new(AtomicU32::new(0));
        let tick_count = Arc::new(AtomicU32::new(0));
        let shutdown_called = Arc::new(AtomicU32::new(0));

        let runnable = CountingRunnable {
            run_count: run_count.clone(),
            tick_count: tick_count.clone(),
            shutdown_called: shutdown_called.clone(),
        };

        // Very short tick interval for testing.
        let mut worker = BackgroundWorker::start(runnable, 20);

        // Wait for a few ticks.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        worker.stop().await;

        // Should have at least 2 ticks in 100ms with 20ms interval.
        assert!(tick_count.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn gc_task_variants() {
        // Verify GcTask enum compiles and is usable.
        let _full = GcTask::RunFull;
        let _map = GcTask::RunMap {
            map_name: "users".to_string(),
        };
    }

    #[tokio::test]
    async fn submit_after_stop_returns_error() {
        let runnable = CountingRunnable {
            run_count: Arc::new(AtomicU32::new(0)),
            tick_count: Arc::new(AtomicU32::new(0)),
            shutdown_called: Arc::new(AtomicU32::new(0)),
        };

        let mut worker = BackgroundWorker::start(runnable, 60_000);
        worker.stop().await;

        let result = worker.submit("late-task".to_string()).await;
        assert!(result.is_err());
    }
}
