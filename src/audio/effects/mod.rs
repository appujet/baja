pub mod crossfade;
pub mod fade;
pub mod tape;
pub mod volume;

use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64},
};

use crate::audio::buffer::PooledBuffer;

/// Interface for transition effects.
pub trait TransitionEffect: Send {
    fn process(
        &mut self,
        mix_buf: &mut [i32],
        i: &mut usize,
        out_len: usize,
        vol: f32,
        stash: &mut Vec<i16>,
        rx: &flume::Receiver<PooledBuffer>,
        state_atomic: &Arc<AtomicU8>,
        position_atomic: &Arc<AtomicU64>,
    ) -> bool;
}
