//! `AudioSource` — the common contract for all readable audio sources.
//!
//! This module defines the [`AudioSource`] trait and provides helpers
//! like [`create_client`] for building HTTP clients used by backends.

pub mod client;
pub mod http;
pub mod segmented;
pub mod traits;

pub use client::create_client;
pub use http::HttpSource;
pub use segmented::SegmentedSource;
pub use traits::AudioSource;
