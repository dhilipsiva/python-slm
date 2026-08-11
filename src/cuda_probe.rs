//! Safe wrapper around the build-time MSVC/CUDA ABI probe.

use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LinkedCudaVersions {
    pub msvc: i32,
    pub cuda_runtime: i32,
    pub cuda_driver: i32,
    pub cublas: i32,
    pub curand: i32,
    pub cudnn: usize,
}

unsafe extern "C" {
    fn rust_llm_msvc_version() -> i32;
    fn rust_llm_cuda_versions(
        device_index: i32,
        runtime_version: *mut i32,
        driver_version: *mut i32,
        cublas_version: *mut i32,
        curand_version: *mut i32,
        cudnn_version: *mut usize,
    ) -> i32;
}

/// Query every native library that `build.rs` linked into this executable.
pub fn linked_cuda_versions(device_index: usize) -> Result<LinkedCudaVersions> {
    let device_index = i32::try_from(device_index)
        .map_err(|_| anyhow::anyhow!("CUDA device index does not fit a C int"))?;
    let mut cuda_runtime = 0;
    let mut cuda_driver = 0;
    let mut cublas = 0;
    let mut curand = 0;
    let mut cudnn = 0;
    // SAFETY: all pointers refer to initialized, live stack values with the exact
    // types declared by the matching `extern "C"` definitions in `kernels/`.
    let (status, msvc) = unsafe {
        (
            rust_llm_cuda_versions(
                device_index,
                &mut cuda_runtime,
                &mut cuda_driver,
                &mut cublas,
                &mut curand,
                &mut cudnn,
            ),
            rust_llm_msvc_version(),
        )
    };
    ensure!(
        status == 0,
        "linked CUDA ABI probe failed with status {status}"
    );
    Ok(LinkedCudaVersions {
        msvc,
        cuda_runtime,
        cuda_driver,
        cublas,
        curand,
        cudnn,
    })
}
