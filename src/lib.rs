//! Decoding of WebP Images
pub use self::decoder::{Error, read_image};

mod decoder;
mod loop_filter;
mod transform;
mod vp8_arithmetic_decoder;
mod vp8_common;
mod vp8_prediction;

pub mod vp8;
