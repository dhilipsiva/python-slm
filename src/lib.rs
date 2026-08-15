#![deny(unsafe_op_in_unsafe_fn)]

pub mod backend;
pub mod config;
#[cfg(feature = "cuda-msvc-link")]
pub mod cuda_probe;
pub mod data;
pub mod model;
pub mod train;
