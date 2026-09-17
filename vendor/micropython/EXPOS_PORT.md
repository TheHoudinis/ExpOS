# MicroPython v1.26.0 source

Source: https://github.com/micropython/micropython/archive/refs/tags/v1.26.0.tar.gz

Archive SHA-256: `3a161ffc3a33f2f326bbddae2991037fa3bb12a781603e4f4bd89a94494f78c2`.
The `py/` core, selected shared support files, two extension headers and
upstream MIT license are retained.
ExpOS's port is in `ports/python/`. ExpPython is its product name; this is
MicroPython, not CPython or a new Python implementation.

Local patch: both generic iterator entry points charge ExpPython's execution
budget. This covers long-running C builtins such as `sum(range(...))` as well
as the VM's ordinary loop/return hooks. Budget abort uses MicroPython's native
VM abort facility and cannot be suppressed by Python exception handlers.
