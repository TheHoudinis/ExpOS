#ifndef EXPOS_FORM_ABI_H
#define EXPOS_FORM_ABI_H

#include <stddef.h>
#include <stdint.h>

#define EXPOS_FORM_ABI_VERSION UINT16_C(1)

typedef struct expos_fin {
    uint8_t bytes[16];
} expos_fin;

typedef enum expos_call {
    EXPOS_CALL_LOG = 1,
    EXPOS_CALL_FORM_RESOLVE = 2,
    EXPOS_CALL_HANDLE_AUTHORIZE = 3,
    EXPOS_CALL_TIME_NOW = 4,
    EXPOS_CALL_IPC_SEND = 5,
    EXPOS_CALL_IPC_RECEIVE = 6,
    EXPOS_CALL_SURFACE_CREATE = 16,
    EXPOS_CALL_BUFFER_ATTACH = 17,
    EXPOS_CALL_SURFACE_DAMAGE = 18,
    EXPOS_CALL_SURFACE_COMMIT = 19,
    EXPOS_CALL_EVENT_POLL = 20,
    EXPOS_CALL_BROWSER_NAVIGATE = 32,
    EXPOS_CALL_STORAGE_READ = 33,
    EXPOS_CALL_STORAGE_WRITE = 34,
    EXPOS_CALL_NETWORK_SEND = 35,
    EXPOS_CALL_NETWORK_RECEIVE = 36,
    EXPOS_CALL_PACKAGE_TRANSACTION = 48
} expos_call;

typedef enum expos_status {
    EXPOS_STATUS_OK = 0,
    EXPOS_STATUS_INVALID = 1,
    EXPOS_STATUS_DENIED = 2,
    EXPOS_STATUS_UNSUPPORTED = 3,
    EXPOS_STATUS_WOULD_BLOCK = 4
} expos_status;

typedef struct expos_request {
    uint16_t version;
    uint16_t call;
    expos_fin caller;
    uint32_t handle;
    uint64_t arguments[6];
} expos_request;

typedef struct expos_response {
    uint16_t status;
    uint8_t reserved[6];
    uint64_t values[4];
} expos_response;

typedef int (*expos_transport)(void *context, const expos_request *request,
                              expos_response *response);

static inline expos_request expos_request_make(expos_call call, expos_fin caller,
                                               uint32_t handle) {
    expos_request request = {0};
    request.version = EXPOS_FORM_ABI_VERSION;
    request.call = (uint16_t)call;
    request.caller = caller;
    request.handle = handle;
    return request;
}

_Static_assert(sizeof(expos_fin) == 16, "ExpOS FIN layout drifted");
_Static_assert(offsetof(expos_request, arguments) == 24,
               "ExpOS request alignment drifted");
_Static_assert(sizeof(expos_request) == 72, "ExpOS request layout drifted");
_Static_assert(sizeof(expos_response) == 40, "ExpOS response layout drifted");

#endif
