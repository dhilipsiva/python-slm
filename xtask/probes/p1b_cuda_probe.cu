
#include <cuda.h>
#include <cuda_runtime.h>
#include <cublas_v2.h>
#include <cublasLt.h>

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace {
constexpr std::uint64_t kAllocationBytes = 2952790016ULL;
constexpr std::uint32_t kSentinel = 42U;
constexpr char kModel[] = "NVIDIA GeForce RTX 5090";

__global__ void p1b_sentinel_kernel(std::uint32_t* words, std::uint64_t last_word) {
    if (blockIdx.x == 0 && threadIdx.x == 0) {
        words[0] = kSentinel;
        words[last_word] = kSentinel;
    }
}

bool uuid_text(const cudaUUID_t& uuid, char* output, std::size_t capacity) {
    const int written = std::snprintf(
        output,
        capacity,
        "GPU-%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        static_cast<unsigned char>(uuid.bytes[0]),
        static_cast<unsigned char>(uuid.bytes[1]),
        static_cast<unsigned char>(uuid.bytes[2]),
        static_cast<unsigned char>(uuid.bytes[3]),
        static_cast<unsigned char>(uuid.bytes[4]),
        static_cast<unsigned char>(uuid.bytes[5]),
        static_cast<unsigned char>(uuid.bytes[6]),
        static_cast<unsigned char>(uuid.bytes[7]),
        static_cast<unsigned char>(uuid.bytes[8]),
        static_cast<unsigned char>(uuid.bytes[9]),
        static_cast<unsigned char>(uuid.bytes[10]),
        static_cast<unsigned char>(uuid.bytes[11]),
        static_cast<unsigned char>(uuid.bytes[12]),
        static_cast<unsigned char>(uuid.bytes[13]),
        static_cast<unsigned char>(uuid.bytes[14]),
        static_cast<unsigned char>(uuid.bytes[15]));
    return written == 40;
}

bool parse_bytes(const char* text, std::uint64_t* value) {
    if (text == nullptr || value == nullptr || *text == '\0') {
        return false;
    }
    char* end = nullptr;
    const unsigned long long parsed = std::strtoull(text, &end, 10);
    if (end == text || end == nullptr || *end != '\0') {
        return false;
    }
    *value = static_cast<std::uint64_t>(parsed);
    return true;
}

int fail(const char* code) {
    std::fprintf(stderr, "%s\n", code);
    return 3;
}
}  // namespace

int main(int argc, char** argv) {
    std::uint64_t allocation_bytes = 0;
    const char* requested_uuid = nullptr;
    for (int index = 1; index < argc; ++index) {
        if (std::strcmp(argv[index], "--allocation-bytes") == 0 && index + 1 < argc) {
            if (!parse_bytes(argv[++index], &allocation_bytes)) {
                return fail("P1B_ALLOCATION_ARGUMENT_INVALID");
            }
        } else if (std::strcmp(argv[index], "--device-uuid") == 0 && index + 1 < argc) {
            requested_uuid = argv[++index];
        } else {
            return fail("P1B_RUNTIME_ARGUMENT_INVALID");
        }
    }
    if (allocation_bytes != kAllocationBytes || allocation_bytes % sizeof(std::uint32_t) != 0) {
        return fail("P1B_ALLOCATION_SIZE_INVALID");
    }

    int device_count = 0;
    if (cudaGetDeviceCount(&device_count) != cudaSuccess || device_count <= 0) {
        return fail("P1B_DEVICE_ENUMERATION_FAILED");
    }
    int selected = -1;
    int matching = 0;
    char selected_uuid[41] = {};
    cudaDeviceProp selected_properties = {};
    for (int device = 0; device < device_count; ++device) {
        cudaDeviceProp properties = {};
        if (cudaGetDeviceProperties(&properties, device) != cudaSuccess) {
            return fail("P1B_DEVICE_IDENTITY_FAILED");
        }
        char candidate_uuid[41] = {};
        if (!uuid_text(properties.uuid, candidate_uuid, sizeof(candidate_uuid))) {
            return fail("P1B_DEVICE_UUID_FORMAT_FAILED");
        }
        if (std::strcmp(properties.name, kModel) == 0 && properties.major == 12 && properties.minor == 0) {
            ++matching;
            if (requested_uuid != nullptr && _stricmp(candidate_uuid, requested_uuid) == 0) {
                selected = device;
                selected_properties = properties;
                std::memcpy(selected_uuid, candidate_uuid, sizeof(selected_uuid));
            } else if (requested_uuid == nullptr) {
                selected = device;
                selected_properties = properties;
                std::memcpy(selected_uuid, candidate_uuid, sizeof(selected_uuid));
            }
        }
    }
    if (matching == 0 || selected < 0) {
        return fail(requested_uuid == nullptr ? "P1B_RTX5090_NOT_FOUND" : "P1B_DEVICE_UUID_NOT_FOUND");
    }
    if (requested_uuid == nullptr && matching != 1) {
        return fail("P1B_RTX5090_AMBIGUOUS");
    }

    cudaStream_t stream = nullptr;
    cublasHandle_t cublas = nullptr;
    cublasLtHandle_t cublaslt = nullptr;
    void* allocation = nullptr;
    bool synchronized = false;
    bool released = false;
    std::size_t free_before = 0;
    std::size_t free_during = 0;
    std::size_t free_after = 0;
    std::size_t total_memory = 0;
    std::uint32_t first = 0;
    std::uint32_t last = 0;
    int runtime_version = 0;
    int driver_version = 0;
    int cublas_version = 0;
    std::size_t cublaslt_version = 0;
    int result = 0;

#define CUDA_REQUIRE(call, code) do { if ((call) != cudaSuccess) { result = fail(code); goto cleanup; } } while (false)
#define CUBLAS_REQUIRE(call, code) do { if ((call) != CUBLAS_STATUS_SUCCESS) { result = fail(code); goto cleanup; } } while (false)

    CUDA_REQUIRE(cudaSetDevice(selected), "P1B_DEVICE_SELECT_FAILED");
    CUDA_REQUIRE(cudaFree(nullptr), "P1B_CONTEXT_CREATE_FAILED");
    CUDA_REQUIRE(cudaRuntimeGetVersion(&runtime_version), "P1B_RUNTIME_VERSION_FAILED");
    CUDA_REQUIRE(cudaDriverGetVersion(&driver_version), "P1B_DRIVER_VERSION_FAILED");
    CUDA_REQUIRE(cudaStreamCreateWithFlags(&stream, cudaStreamNonBlocking), "P1B_STREAM_CREATE_FAILED");
    CUBLAS_REQUIRE(cublasCreate(&cublas), "P1B_CUBLAS_CREATE_FAILED");
    CUBLAS_REQUIRE(cublasSetStream(cublas, stream), "P1B_CUBLAS_STREAM_FAILED");
    CUBLAS_REQUIRE(cublasGetVersion(cublas, &cublas_version), "P1B_CUBLAS_VERSION_FAILED");
    CUBLAS_REQUIRE(cublasLtCreate(&cublaslt), "P1B_CUBLASLT_CREATE_FAILED");
    cublaslt_version = cublasLtGetVersion();
    if (cublaslt_version == 0) {
        result = fail("P1B_CUBLASLT_VERSION_FAILED");
        goto cleanup;
    }
    CUDA_REQUIRE(cudaMemGetInfo(&free_before, &total_memory), "P1B_MEMORY_BEFORE_FAILED");
    CUDA_REQUIRE(cudaMalloc(&allocation, static_cast<std::size_t>(allocation_bytes)), "P1B_ALLOCATION_FAILED");
    CUDA_REQUIRE(cudaMemsetAsync(allocation, 0xA5, static_cast<std::size_t>(allocation_bytes), stream), "P1B_ALLOCATION_TOUCH_FAILED");
    p1b_sentinel_kernel<<<1, 1, 0, stream>>>(
        static_cast<std::uint32_t*>(allocation),
        (allocation_bytes / sizeof(std::uint32_t)) - 1ULL);
    CUDA_REQUIRE(cudaGetLastError(), "P1B_SENTINEL_LAUNCH_FAILED");
    CUDA_REQUIRE(cudaMemcpyAsync(&first, allocation, sizeof(first), cudaMemcpyDeviceToHost, stream), "P1B_SENTINEL_FIRST_READ_FAILED");
    CUDA_REQUIRE(cudaMemcpyAsync(
        &last,
        static_cast<const std::uint32_t*>(allocation) + (allocation_bytes / sizeof(std::uint32_t)) - 1ULL,
        sizeof(last),
        cudaMemcpyDeviceToHost,
        stream), "P1B_SENTINEL_LAST_READ_FAILED");
    CUDA_REQUIRE(cudaStreamSynchronize(stream), "P1B_RUNTIME_SYNCHRONIZE_FAILED");
    synchronized = true;
    CUDA_REQUIRE(cudaMemGetInfo(&free_during, &total_memory), "P1B_MEMORY_DURING_FAILED");
    if (first != kSentinel || last != kSentinel) {
        result = fail("P1B_SENTINEL_MISMATCH");
        goto cleanup;
    }

cleanup:
    if (allocation != nullptr && cudaFree(allocation) != cudaSuccess && result == 0) {
        result = fail("P1B_ALLOCATION_FREE_FAILED");
    }
    allocation = nullptr;
    if (cublaslt != nullptr && cublasLtDestroy(cublaslt) != CUBLAS_STATUS_SUCCESS && result == 0) {
        result = fail("P1B_CUBLASLT_DESTROY_FAILED");
    }
    cublaslt = nullptr;
    if (cublas != nullptr && cublasDestroy(cublas) != CUBLAS_STATUS_SUCCESS && result == 0) {
        result = fail("P1B_CUBLAS_DESTROY_FAILED");
    }
    cublas = nullptr;
    if (stream != nullptr && cudaStreamDestroy(stream) != cudaSuccess && result == 0) {
        result = fail("P1B_STREAM_DESTROY_FAILED");
    }
    stream = nullptr;
    if (cudaDeviceSynchronize() != cudaSuccess && result == 0) {
        result = fail("P1B_CLEANUP_SYNCHRONIZE_FAILED");
    }
    if (cudaMemGetInfo(&free_after, &total_memory) != cudaSuccess && result == 0) {
        result = fail("P1B_MEMORY_AFTER_FAILED");
    }
    released = allocation == nullptr && cublas == nullptr && cublaslt == nullptr && stream == nullptr;
    if (result != 0) {
        return result;
    }

    std::printf(
        "{\"schema\":\"python-slm-p1b-native-runtime-result-v1\",\"status\":\"PASS\","
        "\"device_uuid\":\"%s\",\"device_model\":\"%s\",\"compute_capability\":\"12.0\","
        "\"total_vram_bytes\":%llu,\"allocation_bytes\":%llu,"
        "\"free_memory_before_bytes\":%llu,\"free_memory_during_bytes\":%llu,\"free_memory_after_bytes\":%llu,"
        "\"sentinel_first\":%u,\"sentinel_last\":%u,\"runtime_version\":%d,\"driver_version\":%d,"
        "\"cublas_version\":%d,\"cublaslt_version\":%llu,\"synchronized\":%s,\"owned_resources_released\":%s}\n",
        selected_uuid,
        selected_properties.name,
        static_cast<unsigned long long>(selected_properties.totalGlobalMem),
        static_cast<unsigned long long>(allocation_bytes),
        static_cast<unsigned long long>(free_before),
        static_cast<unsigned long long>(free_during),
        static_cast<unsigned long long>(free_after),
        first,
        last,
        runtime_version,
        driver_version,
        cublas_version,
        static_cast<unsigned long long>(cublaslt_version),
        synchronized ? "true" : "false",
        released ? "true" : "false");
    return 0;
}
