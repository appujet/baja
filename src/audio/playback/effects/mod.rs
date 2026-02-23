use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64},
};

pub mod tape;
use crate::audio::buffer::PooledBuffer;

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
    ) -> bool; // returns true if track contributed audio
}
