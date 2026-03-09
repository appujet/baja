pub mod pool;
pub mod ring;

pub use pool::{BufferPool, get_byte_pool};
pub use ring::RingBuffer;

/// A specialized vector for audio data that can be efficiently pooled.
pub type PooledBuffer = Vec<i16>;

/// Safely casts a `Vec<i16>` to `Vec<u8>` without reallocation.
pub fn cast_to_bytes(v: PooledBuffer) -> Vec<u8> {
    unsafe {
        let (ptr, len, cap) = v.into_raw_parts();
        Vec::from_raw_parts(ptr as *mut u8, len * 2, cap * 2)
    }
}

/// Safely casts a `Vec<u8>` to `Vec<i16>` without reallocation.
pub fn cast_from_bytes(v: Vec<u8>) -> PooledBuffer {
    unsafe {
        let (ptr, len, cap) = v.into_raw_parts();
        Vec::from_raw_parts(ptr as *mut i16, len / 2, cap / 2)
    }
}

/// Returns a byte-slice view of the pooled buffer.
#[inline]
pub fn as_byte_slice(v: &[i16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) }
}

/// Returns an i16-slice view of a byte slice.
#[inline]
pub fn as_i16_slice(v: &[u8]) -> &[i16] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const i16, v.len() / 2) }
}

/// Releases a buffer back to the global pool.
#[inline]
pub fn release_buffer(v: PooledBuffer) {
    get_byte_pool().release(cast_to_bytes(v));
}

/// Acquires a buffer from the global pool.
#[inline]
pub fn acquire_buffer(capacity: usize) -> PooledBuffer {
    cast_from_bytes(get_byte_pool().acquire(capacity * 2))
}
