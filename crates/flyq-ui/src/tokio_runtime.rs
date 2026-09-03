//! Tokio runtime bridge for GPUI.
//!
//! GPUI's own async executor is based on `smol`, but the LanChat network and
//! storage layers are built on Tokio (UDP sockets, timers, libSQL). This module
//! hosts a Tokio runtime as a GPUI [`Global`] so that any view context can spawn
//! real Tokio tasks while still returning a GPUI [`Task`] whose cancellation
//! propagates back to the Tokio side.
//!
//! Based on Zed Industries' `gpui_tokio` crate.

use std::future::Future;

use gpui::{App, AppContext as _, Global, Task};
pub use tokio::task::JoinError;

/// Initialize a fresh multi-threaded Tokio runtime and store it as a global.
///
/// Must be called once inside `Application::run` before any `Tokio::spawn`.
pub fn init(cx: &mut App) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio runtime");
    cx.set_global(GlobalTokio::new(RuntimeHolder::Owned(runtime)));
}

/// Initialize from an existing Tokio [`Handle`] (when a runtime already exists).
#[allow(dead_code)]
pub fn init_from_handle(cx: &mut App, handle: tokio::runtime::Handle) {
    cx.set_global(GlobalTokio::new(RuntimeHolder::Shared(handle)));
}

/// Owns either a self-built runtime or a shared external handle.
enum RuntimeHolder {
    Owned(tokio::runtime::Runtime),
    #[allow(dead_code)]
    Shared(tokio::runtime::Handle),
}

impl RuntimeHolder {
    fn handle(&self) -> &tokio::runtime::Handle {
        match self {
            RuntimeHolder::Owned(runtime) => runtime.handle(),
            RuntimeHolder::Shared(handle) => handle,
        }
    }
}

/// Global wrapper around the Tokio runtime.
struct GlobalTokio {
    runtime: RuntimeHolder,
}

impl Global for GlobalTokio {}

impl GlobalTokio {
    fn new(runtime: RuntimeHolder) -> Self {
        Self { runtime }
    }
}

/// Guard that aborts the underlying Tokio task when dropped.
///
/// Ensures a cancelled GPUI [`Task`] does not leave an orphaned Tokio task
/// running in the background.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Entry point for spawning Tokio work from a GPUI context.
pub struct Tokio;

impl Tokio {
    /// Spawn a future on the Tokio thread pool, returning a GPUI [`Task`].
    ///
    /// Dropping (or cancelling) the returned GPUI task aborts the Tokio task.
    pub fn spawn<T, Fut, R>(cx: &mut gpui::Context<'_, T>, f: Fut) -> Task<Result<R, JoinError>>
    where
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let tokio = cx.global::<GlobalTokio>();
        let join_handle = tokio.runtime.handle().spawn(f);
        let abort_guard = AbortOnDrop(join_handle.abort_handle());
        cx.background_spawn(async move {
            let result = join_handle.await;
            drop(abort_guard);
            result
        })
    }

    /// `anyhow::Result` variant of [`Tokio::spawn`] for convenience.
    #[allow(dead_code)]
    pub fn spawn_result<T, Fut, R>(cx: &mut gpui::Context<'_, T>, f: Fut) -> Task<anyhow::Result<R>>
    where
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let tokio = cx.global::<GlobalTokio>();
        let join_handle = tokio.runtime.handle().spawn(f);
        let abort_guard = AbortOnDrop(join_handle.abort_handle());
        cx.background_spawn(async move {
            let result = join_handle.await;
            drop(abort_guard);
            match result {
                Ok(Ok(r)) => Ok(r),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        })
    }

    /// Get a clone of the Tokio runtime handle (for non-GPUI contexts).
    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        cx.global::<GlobalTokio>().runtime.handle().clone()
    }
}
