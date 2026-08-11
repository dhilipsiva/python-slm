#include <cublas_v2.h>
#include <cuda_runtime_api.h>
#include <cudnn.h>
#include <curand.h>
#include <cstddef>

// This is an ABI/toolchain probe, not an attention implementation. Burn owns the
// actual CUDA execution path. Returning version numbers makes mislinked installs
// diagnosable without Python.
extern "C" int rust_llm_cuda_versions(
    int device_index,
    int* runtime_version,
    int* driver_version,
    int* cublas_version,
    int* curand_version,
    std::size_t* cudnn_version) {
    if (cudaSetDevice(device_index) != cudaSuccess) return 1;
    if (cudaRuntimeGetVersion(runtime_version) != cudaSuccess) return 2;
    if (cudaDriverGetVersion(driver_version) != cudaSuccess) return 3;
    cublasHandle_t handle;
    if (cublasCreate(&handle) != CUBLAS_STATUS_SUCCESS) return 4;
    if (cublasGetVersion(handle, cublas_version) != CUBLAS_STATUS_SUCCESS) {
        cublasDestroy(handle);
        return 5;
    }
    if (cublasDestroy(handle) != CUBLAS_STATUS_SUCCESS) return 6;
    if (curandGetVersion(curand_version) != CURAND_STATUS_SUCCESS) return 7;
    *cudnn_version = cudnnGetVersion();
    return *cudnn_version == 0 ? 8 : 0;
}
