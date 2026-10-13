#ifndef EXPOS_NATIVE_H
#define EXPOS_NATIVE_H
#include "expos.h"

/* EXF1 bootstrap v1: _start(request, response, execute_handle, log_handle).
 * Request/response reside at the fixed admitted ABI page offsets. The loader
 * fills version and caller FIN. Never substitute host pointers into the gate.
 * No libc, floating point, SIMD, TLS, heap, or ordinary return from _start. */
static inline void expos_native_call(volatile expos_request *request,
                                    volatile expos_response *response) {
    uint64_t magic = UINT64_C(0x4558504f53414249);
    __asm__ volatile("int $0x80" : "+a"(magic) : "D"(request), "S"(response) : "memory", "cc");
}

__attribute__((noreturn)) static inline void expos_native_exit(
    volatile expos_request *request, volatile expos_response *response,
    uint32_t execute_handle, uint64_t result) {
    request->call = EXPOS_CALL_EXECUTION_EXIT;
    request->handle = execute_handle;
    request->arguments[0] = result;
    expos_native_call(request, response);
    __builtin_trap();
}
#endif
