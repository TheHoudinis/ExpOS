# ExpOS Python SDK

Host development bindings for the existing Form ABI v1. FINs are stable
16-byte identities. Privileged calls carry a nonzero capability handle.
The explicit little-endian request and response layouts are 72 and 40 bytes,
matching the native Rust ABI on x86-64.

```python
from expos import Client, Emulator, fin

caller = fin(7)
transport = Emulator(caller, handle=3)
client = Client(caller, 3, transport)
surface = client.create_surface(0, 0, 640, 480)
client.attach_buffer(surface, 9, 640, 480)
sequence = client.commit(surface)
```

Run from the repository using `PYTHONPATH=sdk/python`, or install this directory
with Python packaging tools. `make python-sdk` runs the tests without installing
anything. The emulator supports bounded surface creation, buffer attachment,
damage and atomic commits. It checks caller identity and revoked handles;
unimplemented browser/package/Form-resolution services return `UNSUPPORTED`.
The client exposes all ABI call numbers through `invoke` for an eventual real
transport, but there is no live kernel transport in this package yet.

For Python **inside** ExpOS, see [ExpPython](../../docs/PYTHON.md).
