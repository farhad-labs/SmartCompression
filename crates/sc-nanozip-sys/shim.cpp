// Flat C API over nanozip-re's CM coder. Every call builds a fresh model, so calls are
// independent and may run on different threads (the shared lookup tables are built once).
#include <cstdint>
#include <cstring>
#include <new>
#include <vector>

#include "nz_cm.h"

extern "C" {

// Compresses `n` bytes. On success returns 0 and stores the payload length in *out_len;
// returns 1 when `out_cap` is too small, 2 on allocation failure.
int sc_nzcm_encode(const uint8_t* data, uint32_t n, int a_bits, int b_bits, uint8_t* out, uint32_t out_cap,
                   uint32_t* out_len) {
    try {
        NzCmInitAll();
        NzCmDecoder* cm = NzCmCreate(a_bits, b_bits, n < 4096u ? 4096u : n);
        std::vector<uint8_t> payload;
        payload.reserve(n / 2 + 64);
        NzCmEncode(cm, data, n, &payload);
        NzCmDestroy(cm);
        if (payload.size() > out_cap) return 1;
        std::memcpy(out, payload.data(), payload.size());
        *out_len = static_cast<uint32_t>(payload.size());
        return 0;
    } catch (const std::bad_alloc&) {
        return 2;
    } catch (...) {
        return 3;
    }
}

// Decodes exactly `out_n` bytes. `in_len` bounds the work a corrupt stream can cause.
int sc_nzcm_decode(const uint8_t* in, uint32_t in_len, int a_bits, int b_bits, uint8_t* out, uint32_t out_n) {
    try {
        NzCmInitAll();
        NzCmDecoder* cm = NzCmCreate(a_bits, b_bits, out_n < 4096u ? 4096u : out_n);
        NzCmSetBitBudget(cm, static_cast<uint64_t>(out_n) * 8u);
        NzCmDecode(cm, in, in_len, out, out_n);
        NzCmDestroy(cm);
        return 0;
    } catch (const std::bad_alloc&) {
        return 2;
    } catch (...) {
        return 3;
    }
}

}  // extern "C"
