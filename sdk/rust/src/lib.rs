#![cfg_attr(not(test), no_std)]

//! Rust client surface for the frozen ExpOS Form ABI v1.
//!
//! A transport is deliberately supplied by the execution environment. Host
//! tests can use an emulator; native Forms will receive a kernel transport
//! only after the user-mode loader and call gate land.

pub use expos_core::{
    AbiCall as Call, AbiRequest as Request, AbiResponse as Response, AbiStatus as Status, Fin,
    FORM_ABI_VERSION,
};

pub trait Transport {
    type Error;

    fn call(&mut self, request: Request) -> Result<Response, Self::Error>;
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
}
