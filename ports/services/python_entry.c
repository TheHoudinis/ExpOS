#include "expos_native.h"
#include <stddef.h>
#include <stdint.h>
#include <string.h>

static volatile expos_request *request;
static volatile expos_response *response;
static uint32_t execute_handle, log_handle;
extern int expos_python_run(const char *, size_t);

void expos_python_output(const char *text, size_t length) {
    while (length) {
        size_t count = length > 512 ? 512 : length;
        unsigned char *buffer = (unsigned char *)request + 256;
        memcpy(buffer, text, count);
        request->call = EXPOS_CALL_IPC_SEND;
        request->handle = log_handle;
        request->arguments[0] = 256;
        request->arguments[1] = count;
        expos_native_call(request, response);
        if (response->status != EXPOS_STATUS_OK)
            expos_native_exit(request, response, execute_handle, 2);
        text += count;
        length -= count;
    }
}

__attribute__((noreturn)) void expos_python_fatal(void) {
    __builtin_trap();
}

__attribute__((noreturn, section(".text.entry")))
void _start(volatile expos_request *req, volatile expos_response *resp,
            uint32_t execute, uint32_t log, const char *source, size_t length) {
    request = req;
    response = resp;
    execute_handle = execute;
    log_handle = log;
    int result = expos_python_run(source, length);
    expos_native_exit(request, response, execute_handle, (uint64_t)result);
}
