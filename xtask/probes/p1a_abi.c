#include <stdint.h>

uint64_t p1a_c_probe(uint64_t left, uint64_t right) {
    return left + right + UINT64_C(0x0c17);
}
