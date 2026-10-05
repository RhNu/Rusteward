//! Bound independent file work and collect results in discovery order.

use std::thread;

use anyhow::{Context, Result};
use rayon::{
    ThreadPool, ThreadPoolBuilder,
    iter::{IntoParallelRefIterator, ParallelIterator},
};
use tracing::{info, warn};

use crate::config::ExecutionSettings;

/// One invocation owns its pool; sequential phases share the same worker budget.
pub struct Executor {
    pool: Option<ThreadPool>,
    jobs: usize,
}

impl Executor {
    /// Resolve automatic concurrency once and avoid spawning workers for zero or one file.
    ///
    /// # Errors
    /// Returns an error when the requested worker pool cannot be created.
    pub fn new(settings: &ExecutionSettings, files: usize) -> Result<Self> {
        let available = if settings.jobs == 0 && files > 1 {
            thread::available_parallelism().map_or_else(
                |error| {
                    warn!(%error, "cannot determine available CPUs; using one file worker");
                    1
                },
                usize::from,
            )
        } else {
            1
        };
        let jobs = worker_count(settings.jobs, available, files);
        let pool = if jobs > 1 {
            Some(
                ThreadPoolBuilder::new()
                    .num_threads(jobs)
                    .thread_name(|index| format!("rusteward-{index}"))
                    .build()
                    .context("cannot create file worker pool")?,
            )
        } else {
            None
        };
        info!(
            requested_jobs = settings.jobs,
            jobs, files, "configured file execution"
        );
        Ok(Self { pool, jobs })
    }

    /// The maximum number of simultaneously processed files, independent of child pipe threads.
    pub const fn jobs(&self) -> usize { self.jobs }

    /// Finish all parallel work before returning the first error in input order.
    ///
    /// # Errors
    /// Returns the first operation error in input order.
    pub fn map<T, R, F>(&self, items: &[T], operation: F) -> Result<Vec<R>>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> Result<R> + Send + Sync,
    {
        if let Some(pool) = &self.pool {
            // Collect an indexed vector before inspecting errors. Parallel Result collection
            // could instead select whichever failure happens to finish first.
            let results: Vec<Result<R>> =
                pool.install(|| items.par_iter().map(operation).collect());
            results.into_iter().collect()
        } else {
            items.iter().map(operation).collect()
        }
    }
}

/// Explicit requests may exceed available CPUs but never exceed the number of source files.
fn worker_count(requested: usize, available: usize, files: usize) -> usize {
    let budget = if requested == 0 {
        available.max(1)
    } else {
        requested
    };
    budget.min(files)
}

#[cfg(test)]
mod tests;
