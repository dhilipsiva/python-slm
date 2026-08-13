#include <cstdint>

extern "C" std::uint64_t p1a_cpp_probe(std::uint64_t left,
                                        std::uint64_t right) noexcept {
    return left * UINT64_C(3) + right * UINT64_C(5) + UINT64_C(20);
}
