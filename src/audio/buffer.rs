use std::sync::{Arc, Mutex, OnceLock};
use std::ops::{Deref, DerefMut};

pub type SharedBufferPool = Arc<BufferPoolInner>;

pub struct BufferPoolInner {
    pool: Mutex<Vec<Vec<i16>>>,
    buffer_size: usize,
}

impl BufferPoolInner {
    pub fn new(buffer_size: usize) -> Arc<Self> {
        Arc::new(Self {
            pool: Mutex::new(Vec::with_capacity(64)),
            buffer_size,
        })
    }

    pub fn acquire(self: &Arc<Self>) -> PooledBuffer {
        let mut pool = self.pool.lock().unwrap();
        let mut vec = pool.pop().unwrap_or_else(|| Vec::with_capacity(self.buffer_size));
        vec.clear();
        PooledBuffer {
            vec: Some(vec),
            pool: Some(self.clone()),
        }
    }

    fn release(&self, mut vec: Vec<i16>) {
        let mut pool = self.pool.lock().unwrap();
        if pool.len() < 128 {
            vec.clear();
            pool.push(vec);
        }
    }
}

pub struct PooledBuffer {
    vec: Option<Vec<i16>>,
    pool: Option<Arc<BufferPoolInner>>,
}

impl Deref for PooledBuffer {
    type Target = Vec<i16>;
    fn deref(&self) -> &Self::Target {
        self.vec.as_ref().unwrap()
    }
}

impl DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.vec.as_mut().unwrap()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let (Some(vec), Some(pool)) = (self.vec.take(), self.pool.take()) {
            pool.release(vec);
        }
    }
}

pub static GLOBAL_BUFFER_POOL: OnceLock<Arc<BufferPoolInner>> = OnceLock::new();

pub fn get_pool() -> Arc<BufferPoolInner> {
    GLOBAL_BUFFER_POOL
        .get_or_init(|| BufferPoolInner::new(4096))
        .clone()
}

