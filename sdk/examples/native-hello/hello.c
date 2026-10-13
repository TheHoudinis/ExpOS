#include "expos_native.h"

static char greeting[] = "Hello from compiled C in Ring 3";
static volatile uint64_t scratch;

__attribute__((noreturn, section(".text.entry")))
void _start(volatile expos_request *request, volatile expos_response *response,
            uint32_t execute_handle, uint32_t log_handle) {
    /* Exercise initialized data, zero-filled BSS, and writable user storage. */
    if (scratch != 0)
        expos_native_exit(request, response, execute_handle, 99);
    scratch = 42;
    request->call = EXPOS_CALL_LOG;
    request->handle = log_handle;
    request->arguments[0] = (uintptr_t)greeting - (uintptr_t)request;
    request->arguments[1] = sizeof(greeting) - 1;
    expos_native_call(request, response);
    expos_native_exit(request, response, execute_handle,
                      response->status == EXPOS_STATUS_OK ? scratch : 98);
}
