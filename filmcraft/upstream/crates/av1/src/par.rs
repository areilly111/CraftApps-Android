//! Optional data parallelism (rayon behind the `threads` feature; sequential otherwise and on
//! wasm32).

#[cfg(feature = "threads")]
use std::sync::Arc;

/// A handle to the decoder's worker pool (or none: run on the calling thread).
#[derive(Clone, Default)]
pub(crate) struct Pool {
    #[cfg(feature = "threads")]
    pool: Option<Arc<rayon::ThreadPool>>,
}

impl Pool {
    /// A pool of `threads` workers (sequential when `threads` <= 1, without the `threads`
    /// feature, or on wasm32).
    pub fn new(threads: usize) -> Pool {
        #[cfg(feature = "threads")]
        {
            let pool = if threads > 1 && cfg!(not(target_arch = "wasm32")) {
                rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("av1-{i}")).build().ok().map(Arc::new)
            } else {
                None
            };
            Pool { pool }
        }
        #[cfg(not(feature = "threads"))]
        {
            let _ = threads;
            Pool {}
        }
    }

    pub fn is_parallel(&self) -> bool {
        #[cfg(feature = "threads")]
        {
            self.pool.is_some()
        }
        #[cfg(not(feature = "threads"))]
        {
            false
        }
    }

    /// `f(i, &mut items[i])` for every item, in parallel when possible; results in order.
    pub fn map_mut<T: Send, R: Send>(&self, items: &mut [T], f: impl Fn(usize, &mut T) -> R + Sync + Send) -> Vec<R> {
        #[cfg(feature = "threads")]
        if let Some(pool) = &self.pool
            && items.len() > 1
        {
            use rayon::prelude::*;
            return pool.install(|| items.par_iter_mut().enumerate().map(|(i, t)| f(i, t)).collect());
        }
        items.iter_mut().enumerate().map(|(i, t)| f(i, t)).collect()
    }

    /// `f(i, chunk)` for the consecutive `chunk_len`-element chunks of `data`.
    pub fn chunks<T: Send>(&self, data: &mut [T], chunk_len: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
        let chunk_len = chunk_len.max(1);
        #[cfg(feature = "threads")]
        if let Some(pool) = &self.pool
            && data.len() > chunk_len
        {
            use rayon::prelude::*;
            pool.install(|| data.par_chunks_mut(chunk_len).enumerate().for_each(|(i, c)| f(i, c)));
            return;
        }
        for (i, c) in data.chunks_mut(chunk_len).enumerate() {
            f(i, c);
        }
    }
}
