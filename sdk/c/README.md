# ExpOS C SDK

`expos.h` freezes the C representation of ExpOS Form ABI v1 and checks its
layout at compile time. It deliberately exposes a capability Handle and a
transport callback—not files, device paths, descriptors, or POSIX syscalls.

```c
#include <expos.h>

expos_request request = expos_request_make(EXPOS_CALL_SURFACE_CREATE,
                                            caller_fin, display_handle);
request.arguments[2] = 640;
request.arguments[3] = 480;
```

The x86_64 kernel has a native shared-page/interrupt transport for its bounded
executable capsule. Loading a general C artifact into that capsule format is
not implemented yet. Run `make c-sdk` from the repository root to compile the
contract test.
