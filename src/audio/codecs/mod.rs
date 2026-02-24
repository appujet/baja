pub mod opus;

use symphonia::core::codecs::CodecRegistry;

/// Register all custom codecs into the given registry.
pub fn register_codecs(registry: &mut CodecRegistry) {
    registry.register_all::<opus::OpusCodecDecoder>();
}
