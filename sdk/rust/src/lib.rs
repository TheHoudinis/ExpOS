#![cfg_attr(not(test), no_std)]

//! Rust client surface for the frozen ExpOS Form ABI v1.
//!
//! A transport is deliberately supplied by the execution environment. Host
//! tests can use an emulator. The kernel's bounded x86_64 capsule uses a
//! validated shared-page/interrupt transport; general Rust artifact loading is
//! a separate integration layer.

pub use expos_core::{
    AbiCall as Call, AbiRequest as Request, AbiResponse as Response, AbiStatus as Status, Fin,
    StreamCall, StreamRequest, FORM_ABI_VERSION, STREAM_ABI_VERSION, STREAM_CHUNK_BYTES,
};

pub trait Transport {
    type Error;

    fn call(&mut self, request: Request) -> Result<Response, Self::Error>;
}

/// Separate transport for the additive blob-stream gate. Implementations copy
/// `transfer` through the admitted ABI-page window; v1 `Transport` is unchanged.
pub trait StreamTransport {
    type Error;

    fn stream_call(
        &mut self,
        request: StreamRequest,
        transfer: &mut [u8],
    ) -> Result<Response, Self::Error>;
}

pub struct StreamClient<T> {
    caller: Fin,
    handle: u32,
    stream: u32,
    transport: T,
}

impl<T: StreamTransport> StreamClient<T> {
    pub fn new(caller: Fin, handle: u32, transport: T) -> Option<Self> {
        (!caller.is_zero() && handle != 0).then_some(Self {
            caller,
            handle,
            stream: 0,
            transport,
        })
    }

    fn invoke(
        &mut self,
        call: StreamCall,
        offset: u64,
        transfer: &mut [u8],
    ) -> Result<Response, Error<T::Error>> {
        if transfer.len() > STREAM_CHUNK_BYTES {
            return Err(Error::Status(Status::Invalid));
        }
        let response = self
            .transport
            .stream_call(
                StreamRequest {
                    version: STREAM_ABI_VERSION,
                    call: call as u16,
                    caller: self.caller,
                    handle_id: self.handle,
                    stream_id: self.stream,
                    offset,
                    length: transfer.len() as u32,
                    flags: 0,
                },
                transfer,
            )
            .map_err(Error::Transport)?;
        let status =
            Status::from_raw(response.status).ok_or(Error::UnknownStatus(response.status))?;
        if status != Status::Ok {
            return Err(Error::Status(status));
        }
        Ok(response)
    }

    pub fn open_read(&mut self) -> Result<usize, Error<T::Error>> {
        let response = self.invoke(StreamCall::OpenRead, 0, &mut [])?;
        self.stream = response.values[0] as u32;
        Ok(response.values[1] as usize)
    }

    pub fn open_write(&mut self) -> Result<usize, Error<T::Error>> {
        let response = self.invoke(StreamCall::OpenWrite, 0, &mut [])?;
        self.stream = response.values[0] as u32;
        Ok(response.values[1] as usize)
    }

    pub fn read(&mut self, offset: u64, output: &mut [u8]) -> Result<usize, Error<T::Error>> {
        Ok(self.invoke(StreamCall::ReadChunk, offset, output)?.values[0] as usize)
    }

    pub fn write(&mut self, offset: u64, bytes: &mut [u8]) -> Result<usize, Error<T::Error>> {
        Ok(self.invoke(StreamCall::WriteChunk, offset, bytes)?.values[0] as usize)
    }

    pub fn commit(&mut self) -> Result<u32, Error<T::Error>> {
        let response = self.invoke(StreamCall::Commit, 0, &mut [])?;
        self.stream = 0;
        Ok(response.values[0] as u32)
    }

    pub fn abort(&mut self) -> Result<(), Error<T::Error>> {
        self.invoke(StreamCall::Abort, 0, &mut [])?;
        self.stream = 0;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error<E> {
    Transport(E),
    Status(Status),
    UnknownStatus(u16),
}

pub struct Client<T> {
    caller: Fin,
    handle: u32,
    transport: T,
}

impl<T: Transport> Client<T> {
    pub fn new(caller: Fin, handle: u32, transport: T) -> Option<Self> {
        if caller.is_zero() || handle == 0 {
            return None;
        }
        Some(Self {
            caller,
            handle,
            transport,
        })
    }

    pub fn invoke(&mut self, call: Call, arguments: [u64; 6]) -> Result<Response, Error<T::Error>> {
        let response = self
            .transport
            .call(Request {
                version: FORM_ABI_VERSION,
                call: call as u16,
                caller: self.caller,
                handle_id: self.handle,
                arguments,
            })
            .map_err(Error::Transport)?;
        let status =
            Status::from_raw(response.status).ok_or(Error::UnknownStatus(response.status))?;
        if status != Status::Ok {
            return Err(Error::Status(status));
        }
        Ok(response)
    }

    pub fn resolve(&mut self, name_token: u64) -> Result<Response, Error<T::Error>> {
        self.invoke(Call::FormResolve, [name_token, 0, 0, 0, 0, 0])
    }

    pub fn authorize(&mut self, operation_bits: u64) -> Result<Response, Error<T::Error>> {
        self.invoke(Call::HandleAuthorize, [operation_bits, 0, 0, 0, 0, 0])
    }

    pub fn time_now(&mut self, clock: u64) -> Result<u64, Error<T::Error>> {
        Ok(self.invoke(Call::TimeNow, [clock, 0, 0, 0, 0, 0])?.values[0])
    }

    pub fn yield_execution(&mut self) -> Result<(), Error<T::Error>> {
        self.invoke(Call::ExecutionYield, [0; 6]).map(|_| ())
    }

    pub fn exit_execution(&mut self, result: u64) -> Result<(), Error<T::Error>> {
        self.invoke(Call::ExecutionExit, [result, 0, 0, 0, 0, 0])
            .map(|_| ())
    }

    pub fn create_surface(
        &mut self,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        role: u16,
    ) -> Result<u32, Error<T::Error>> {
        let response = self.invoke(
            Call::SurfaceCreate,
            [
                x as u16 as u64,
                y as u16 as u64,
                width as u64,
                height as u64,
                role as u64,
                0,
            ],
        )?;
        Ok(response.values[0] as u32)
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingTransport {
        last: Option<Request>,
    }

    impl Transport for RecordingTransport {
        type Error = ();

        fn call(&mut self, request: Request) -> Result<Response, Self::Error> {
            self.last = Some(request);
            Ok(Response {
                status: Status::Ok as u16,
                values: [41, 0, 0, 0],
            })
        }
    }

    #[test]
    fn client_emits_the_frozen_v1_layout() {
        let transport = RecordingTransport { last: None };
        let mut client = Client::new(Fin::from_u128(7), 9, transport).unwrap();
        assert_eq!(client.time_now(1), Ok(41));
        let transport = client.into_transport();
        let request = transport.last.unwrap();
        assert_eq!(request.version, FORM_ABI_VERSION);
        assert_eq!(request.call, Call::TimeNow as u16);
        assert_eq!(request.handle_id, 9);
        assert_eq!(core::mem::size_of::<Request>(), 72);
        assert_eq!(core::mem::size_of::<Response>(), 40);
    }

    #[test]
    fn client_rejects_missing_identity_or_handle() {
        let make_transport = || RecordingTransport { last: None };
        assert!(Client::new(Fin::ZERO, 9, make_transport()).is_none());
        assert!(Client::new(Fin::from_u128(7), 0, make_transport()).is_none());
    }

    struct RecordingStream {
        last: Option<StreamRequest>,
    }

    impl StreamTransport for RecordingStream {
        type Error = ();

        fn stream_call(
            &mut self,
            request: StreamRequest,
            _transfer: &mut [u8],
        ) -> Result<Response, Self::Error> {
            self.last = Some(request);
            Ok(Response::ok([17, 9000, 0, 0]))
        }
    }

    #[test]
    fn stream_client_uses_v2_without_changing_v1() {
        let transport = RecordingStream { last: None };
        let mut client = StreamClient::new(Fin::from_u128(7), 9, transport).unwrap();
        assert_eq!(client.open_read(), Ok(9000));
        let mut output = [0; 32];
        assert_eq!(client.read(4096, &mut output), Ok(17));
        assert_eq!(FORM_ABI_VERSION, 1);
        assert_eq!(STREAM_ABI_VERSION, 2);
    }
}
