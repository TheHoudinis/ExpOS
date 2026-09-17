"""Versioned ExpOS Form ABI. Native transport remains an explicit dependency."""
from dataclasses import dataclass, field
from enum import IntEnum
import struct
from typing import Protocol

ABI_VERSION = 1
REQUEST = struct.Struct("<HH16sI6Q")
RESPONSE = struct.Struct("<H6x4Q")


class Call(IntEnum):
    LOG = 1
    FORM_RESOLVE = 2
    HANDLE_AUTHORIZE = 3
    SURFACE_CREATE = 16
    BUFFER_ATTACH = 17
    SURFACE_DAMAGE = 18
    SURFACE_COMMIT = 19
    EVENT_POLL = 20
    BROWSER_NAVIGATE = 32
    PACKAGE_TRANSACTION = 48


class Status(IntEnum):
    OK = 0
    INVALID = 1
    DENIED = 2
    UNSUPPORTED = 3
    WOULD_BLOCK = 4


def _unsigned(value: int, bits: int) -> bool:
    return type(value) is int and 0 <= value < 1 << bits


def fin(value: int) -> bytes:
    if not _unsigned(value, 128) or value == 0:
        raise ValueError("FIN must be a nonzero 128-bit identity")
    return value.to_bytes(16, "big")


@dataclass(frozen=True)
class Request:
    call: Call
    caller: bytes
    handle: int
    arguments: tuple[int, ...] = (0,) * 6
    version: int = ABI_VERSION

    def __post_init__(self):
        if type(self.caller) is not bytes or len(self.caller) != 16 or not any(self.caller):
            raise ValueError("invalid caller FIN")
        if not _unsigned(self.handle, 32) or not _unsigned(self.version, 16):
            raise ValueError("invalid handle or ABI version")
        if len(self.arguments) != 6 or not all(_unsigned(v, 64) for v in self.arguments):
            raise ValueError("ABI requires six unsigned 64-bit arguments")
        object.__setattr__(self, "call", Call(self.call))
        object.__setattr__(self, "arguments", tuple(self.arguments))

    def pack(self) -> bytes:
        return REQUEST.pack(self.version, self.call, self.caller, self.handle, *self.arguments)

    @classmethod
    def unpack(cls, data: bytes):
        version, call, caller, handle, *args = REQUEST.unpack(data)
        return cls(Call(call), caller, handle, tuple(args), version)


@dataclass(frozen=True)
class Response:
    status: Status
    values: tuple[int, ...] = (0,) * 4

    def __post_init__(self):
        object.__setattr__(self, "status", Status(self.status))
        if len(self.values) != 4 or not all(_unsigned(v, 64) for v in self.values):
            raise ValueError("ABI requires four unsigned 64-bit response values")
        object.__setattr__(self, "values", tuple(self.values))

    def pack(self) -> bytes:
        return RESPONSE.pack(self.status, *self.values)

    @classmethod
    def unpack(cls, data: bytes):
        status, *values = RESPONSE.unpack(data)
        return cls(Status(status), tuple(values))


class Transport(Protocol):
    def call(self, request: Request) -> Response: ...


class ABIError(RuntimeError):
    def __init__(self, call: Call, status: Status):
        super().__init__(f"{call.name}: {status.name}")
        self.status = status


@dataclass
class Client:
    caller: bytes
    handle: int
    transport: Transport

    def invoke(self, call: Call, *arguments: int) -> Response:
        if len(arguments) > 6:
            raise ValueError("at most six arguments")
        request = Request(call, self.caller, self.handle, tuple(arguments) + (0,) * (6 - len(arguments)))
        response = self.transport.call(request)
        if response.status != Status.OK:
            raise ABIError(call, response.status)
        return response

    def create_surface(self, x: int, y: int, width: int, height: int, role: int = 0) -> int:
        if not (-32768 <= x <= 32767 and -32768 <= y <= 32767 and 0 < width <= 65535 and 0 < height <= 65535):
            raise ValueError("invalid surface rectangle")
        return self.invoke(Call.SURFACE_CREATE, x & 65535, y & 65535, width, height, role).values[0]

    def attach_buffer(self, surface: int, buffer: int, width: int, height: int):
        self.invoke(Call.BUFFER_ATTACH, surface, buffer, width, height)

    def damage(self, surface: int, x: int, y: int, width: int, height: int):
        if not (-32768 <= x <= 32767 and -32768 <= y <= 32767 and 0 < width <= 65535 and 0 < height <= 65535):
            raise ValueError("invalid damage rectangle")
        self.invoke(Call.SURFACE_DAMAGE, surface, x & 65535, y & 65535, width, height)

    def commit(self, surface: int) -> int:
        return self.invoke(Call.SURFACE_COMMIT, surface).values[0]

    def resolve(self, identity: int):
        return self.invoke(Call.FORM_RESOLVE, identity)

    def authorize(self, operation: int):
        return self.invoke(Call.HANDLE_AUTHORIZE, operation)

    def navigate(self, resource: int):
        self.invoke(Call.BROWSER_NAVIGATE, resource)

    def poll(self):
        return self.invoke(Call.EVENT_POLL)

    def package_transaction(self, transaction: int):
        return self.invoke(Call.PACKAGE_TRANSACTION, transaction)


@dataclass
class _Surface:
    width: int
    height: int
    pending: int = 0
    current: int = 0


@dataclass
class Emulator:
    """Bounded surface emulator. Unsupported services fail explicitly."""
    caller: bytes
    handle: int
    capacity: int = 16
    revoked: bool = False
    sequence: int = 0
    surfaces: dict[int, _Surface] = field(default_factory=dict)

    def __post_init__(self):
        Request(Call.LOG, self.caller, self.handle)
        if self.handle == 0 or not 1 <= self.capacity <= 256:
            raise ValueError("emulator requires a nonzero handle and bounded capacity")

    def call(self, request: Request) -> Response:
        if request.version != ABI_VERSION or request.caller != self.caller:
            return Response(Status.INVALID)
        if request.call != Call.LOG and (self.revoked or not request.handle or request.handle != self.handle):
            return Response(Status.DENIED)
        args = request.arguments
        if request.call == Call.LOG:
            return Response(Status.OK)
        if request.call == Call.SURFACE_CREATE:
            if len(self.surfaces) >= self.capacity:
                return Response(Status.WOULD_BLOCK)
            if not (0 < args[2] <= 65535 and 0 < args[3] <= 65535 and args[0] <= 65535 and args[1] <= 65535):
                return Response(Status.INVALID)
            surface = len(self.surfaces) + 1
            self.surfaces[surface] = _Surface(args[2], args[3])
            return Response(Status.OK, (surface, 0, 0, 0))
        if request.call in (Call.BUFFER_ATTACH, Call.SURFACE_DAMAGE, Call.SURFACE_COMMIT):
            surface = self.surfaces.get(args[0])
            if surface is None:
                return Response(Status.INVALID)
            if request.call == Call.BUFFER_ATTACH:
                if not (0 < args[1] <= 0xFFFFFFFF and args[2] == surface.width and args[3] == surface.height):
                    return Response(Status.INVALID)
                surface.pending = args[1]
            elif request.call == Call.SURFACE_DAMAGE:
                if not (0 < args[3] <= surface.width and 0 < args[4] <= surface.height):
                    return Response(Status.INVALID)
            else:
                if not surface.pending:
                    return Response(Status.INVALID)
                surface.current = surface.pending
                self.sequence += 1
                return Response(Status.OK, (self.sequence, 0, 0, 0))
            return Response(Status.OK)
        if request.call == Call.EVENT_POLL:
            return Response(Status.WOULD_BLOCK)
        return Response(Status.UNSUPPORTED)
