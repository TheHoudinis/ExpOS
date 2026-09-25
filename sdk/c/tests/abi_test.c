#include "expos.h"

#include <assert.h>

int main(void) {
    expos_fin caller = {{0}};
    caller.bytes[15] = 7;
    expos_request request = expos_request_make(EXPOS_CALL_SURFACE_CREATE, caller, 9);
    request.arguments[2] = 640;
    request.arguments[3] = 480;

    assert(request.version == 1);
    assert(request.call == 16);
    assert(request.handle == 9);
    assert(request.arguments[2] == 640);
    assert(EXPOS_CALL_NETWORK_RECEIVE == 36);
    return 0;
}
