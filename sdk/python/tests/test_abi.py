import unittest
from expos import ABIError, Call, Client, Emulator, Request, Response, Status, fin


class ABIContract(unittest.TestCase):
    def setUp(self):
        self.caller = fin(7)
        self.emulator = Emulator(self.caller, 3)
        self.client = Client(self.caller, 3, self.emulator)

    def test_rust_wire_layout_roundtrip(self):
        request = Request(Call.SURFACE_CREATE, self.caller, 3, (65535, 2, 640, 480, 0, 0))
        self.assertEqual(len(request.pack()), 72)
        self.assertEqual(request.pack()[:4], b'\x01\x00\x10\x00')
        self.assertEqual(Request.unpack(request.pack()), request)
        response = Response(Status.OK, (42, 0, 0, 0))
        self.assertEqual(len(response.pack()), 40)
        self.assertEqual(Response.unpack(response.pack()), response)

    def test_pending_buffer_is_invisible_until_commit(self):
        surface = self.client.create_surface(-1, 2, 640, 480)
        self.client.attach_buffer(surface, 9, 640, 480)
        self.assertEqual(self.emulator.surfaces[surface].current, 0)
        self.assertEqual(self.client.commit(surface), 1)
        self.assertEqual(self.emulator.surfaces[surface].current, 9)
        with self.assertRaises(ABIError):
            self.client.attach_buffer(surface, 10, 1920, 1080)
        self.assertEqual(self.emulator.surfaces[surface].pending, 9)

    def test_authority_cannot_be_invented_or_reused_after_revocation(self):
        with self.assertRaises(ABIError):
            Client(fin(8), 3, self.emulator).create_surface(0, 0, 32, 32)
        with self.assertRaises(ABIError):
            Client(self.caller, 0, self.emulator).create_surface(0, 0, 32, 32)
        self.emulator.revoked = True
        with self.assertRaises(ABIError):
            self.client.create_surface(0, 0, 32, 32)
        self.assertEqual(self.emulator.surfaces, {})

    def test_unsupported_services_and_exhaustion_are_not_faked(self):
        with self.assertRaises(ABIError) as result:
            self.client.package_transaction(1)
        self.assertEqual(result.exception.status, Status.UNSUPPORTED)
        for _ in range(16):
            self.client.create_surface(0, 0, 1, 1)
        with self.assertRaises(ABIError) as result:
            self.client.create_surface(0, 0, 1, 1)
        self.assertEqual(result.exception.status, Status.WOULD_BLOCK)

    def test_bad_wire_values_fail_before_transport(self):
        for identity in (b'', b'\0' * 16, b'1' * 17):
            with self.assertRaises(ValueError):
                Request(Call.LOG, identity, 1)
        with self.assertRaises(ValueError):
            Request(Call.LOG, self.caller, 1, (-1, 0, 0, 0, 0, 0))


if __name__ == '__main__':
    unittest.main()
