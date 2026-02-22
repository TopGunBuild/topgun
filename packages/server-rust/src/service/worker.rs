// Stub file -- will be fully implemented in G4 (SPEC-059d).

use async_trait::async_trait;

/// Trait for tasks executed by `BackgroundWorker`.
#[async_trait]
pub trait BackgroundRunnable: Send + 'static {
    type Task: Send + 'static;
    async fn run(&mut self, task: Self::Task);
    async fn on_tick(&mut self) {}
    async fn shutdown(&mut self) {}
}

/// Generic background worker that processes tasks via an mpsc channel.
pub struct BackgroundWorker<T> {
    _phantom: std::marker::PhantomData<T>,
}
