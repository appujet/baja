use parking_lot::Mutex;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, OnceLock};

pub type SharedBufferPool = Arc<BufferPoolInner>;

pub struct BufferPoolInner {
    pool: Mutex<Vec<Vec<i16>>>,
    buffer_size: usize,
    max_buffers: usize,
}

impl BufferPoolInner {
    pub fn new(buffer_size: usize) -> Arc<Self> {
        Arc::new(Self {
            pool: Mutex::new(Vec::with_capacity(64)),
            buffer_size,
            max_buffers: 128,
        })
    }

    pub fn acquire(self: &Arc<Self>) -> PooledBuffer {
        let mut pool = self.pool.lock();

        let mut vec = pool
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(self.buffer_size));

        vec.clear();

        PooledBuffer {
            vec,
            pool: Arc::clone(self),
        }
    }

    fn release(&self, mut vec: Vec<i16>) {
        let mut pool = self.pool.lock();

        if pool.len() < self.max_buffers {
            vec.clear();
            pool.push(vec);
        }
        // else: drop automatically
    }
}

pub struct PooledBuffer {
    vec: Vec<i16>,
    pool: Arc<BufferPoolInner>,
}

impl Deref for PooledBuffer {
    type Target = Vec<i16>;

    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}

impl DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.vec
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        let vec = std::mem::take(&mut self.vec);
        self.pool.release(vec);
    }
}

pub static GLOBAL_BUFFER_POOL: OnceLock<Arc<BufferPoolInner>> = OnceLock::new();

pub fn get_pool() -> Arc<BufferPoolInner> {
    GLOBAL_BUFFER_POOL
        .get_or_init(|| BufferPoolInner::new(4096))
        .clone()
}