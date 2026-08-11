#include <cstdint>

extern "C" std::int32_t rust_llm_msvc_version() {
    return _MSC_VER;
}

