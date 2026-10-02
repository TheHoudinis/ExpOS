use crate::{CapabilityBroker, CfcFin, Fin, Operations};

/// The stable, language-neutral contract between executable Forms and ExpOS.
/// Kernel releases may change freely behind this boundary, but they must not
/// reinterpret an existing version's call numbers or wire layouts.
pub const FORM_ABI_VERSION: u16 = 1;

/// Compatibility name retained for source written against the first Go SDK.
pub const GO_ABI_VERSION: u16 = FORM_ABI_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum AbiCall {
    Log = 1,
    FormResolve = 2,
    HandleAuthorize = 3,
    TimeNow = 4,
    IpcSend = 5,
    IpcReceive = 6,
    ExecutionYield = 7,
    ExecutionExit = 8,
    SurfaceCreate = 16,
    BufferAttach = 17,
    SurfaceDamage = 18,
    SurfaceCommit = 19,
    EventPoll = 20,
    BrowserNavigate = 32,
    StorageRead = 33,
    StorageWrite = 34,
    NetworkSend = 35,
    NetworkReceive = 36,
    PackageTransaction = 48,
}

impl AbiCall {
    pub const fn from_raw(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::Log,
            2 => Self::FormResolve,
            3 => Self::HandleAuthorize,
            4 => Self::TimeNow,
            5 => Self::IpcSend,
            6 => Self::IpcReceive,
            7 => Self::ExecutionYield,
            8 => Self::ExecutionExit,
            16 => Self::SurfaceCreate,
            17 => Self::BufferAttach,
            18 => Self::SurfaceDamage,
            19 => Self::SurfaceCommit,
            20 => Self::EventPoll,
            32 => Self::BrowserNavigate,
            33 => Self::StorageRead,
            34 => Self::StorageWrite,
            35 => Self::NetworkSend,
            36 => Self::NetworkReceive,
            48 => Self::PackageTransaction,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AbiRequest {
    pub version: u16,
    /// Kept raw at the wire boundary so an unknown future number is data, not
    /// an invalid Rust enum discriminant.
    pub call: u16,
    pub caller: Fin,
    pub handle_id: u32,
    pub arguments: [u64; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AbiResponse {
    /// Kept raw for the same forward-compatibility reason as `AbiRequest.call`.
    pub status: u16,
    pub values: [u64; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum AbiStatus {
    Ok = 0,
    Invalid = 1,
    Denied = 2,
    Unsupported = 3,
    WouldBlock = 4,
}

impl AbiStatus {
    pub const fn from_raw(value: u16) -> Option<Self> {
        Some(match value {
            0 => Self::Ok,
            1 => Self::Invalid,
            2 => Self::Denied,
            3 => Self::Unsupported,
            4 => Self::WouldBlock,
            _ => return None,
        })
    }
}

impl AbiRequest {
    pub fn validate(self) -> Result<Operations, AbiStatus> {
        if self.version != FORM_ABI_VERSION || self.caller.is_zero() {
            return Err(AbiStatus::Invalid);
        }
        let Some(call) = AbiCall::from_raw(self.call) else {
            return Err(AbiStatus::Unsupported);
        };
        let operation = match call {
            AbiCall::Log | AbiCall::FormResolve | AbiCall::TimeNow => Operations::READ,
            AbiCall::EventPoll => Operations::INPUT,
            AbiCall::HandleAuthorize
            | AbiCall::IpcSend
            | AbiCall::IpcReceive
            | AbiCall::ExecutionYield
            | AbiCall::ExecutionExit => Operations::EXECUTE,
            AbiCall::SurfaceCreate
            | AbiCall::BufferAttach
            | AbiCall::SurfaceDamage
            | AbiCall::SurfaceCommit => Operations::DISPLAY,
            AbiCall::BrowserNavigate => Operations::EXECUTE,
            AbiCall::StorageRead => Operations::READ,
            AbiCall::StorageWrite => Operations::CONFIGURE,
            AbiCall::NetworkSend | AbiCall::NetworkReceive => Operations::NETWORK,
            AbiCall::PackageTransaction => Operations::PACKAGE,
        };
        if self.handle_id == 0 && !matches!(call, AbiCall::Log) {
            return Err(AbiStatus::Denied);
        }
        Ok(operation)
    }
}

/// Native kernel call-gate for the frozen Form ABI v1 wire contract.
///
/// The request remains the same 72-byte language-neutral record used by the
/// SDKs.  The gate adds the missing runtime boundary: it derives the required
/// operation from the frozen call number, binds the request to one CFC,
/// Dimension and target Form, and validates the caller-owned Form Handle
/// before dispatching any service implementation.
pub struct NativeCallGate<'a> {
    broker: &'a CapabilityBroker,
    cfc: CfcFin,
    target: Fin,
    dimension: Fin,
}

impl<'a> NativeCallGate<'a> {
    pub const fn new(
        broker: &'a CapabilityBroker,
        cfc: CfcFin,
        target: Fin,
        dimension: Fin,
    ) -> Self {
        Self {
            broker,
            cfc,
            target,
            dimension,
        }
    }

    pub fn dispatch(
        &self,
        request: AbiRequest,
        tick: u64,
        service: impl FnOnce(AbiCall, [u64; 6]) -> AbiResponse,
    ) -> AbiResponse {
        let operation = match request.validate() {
            Ok(operation) => operation,
            Err(status) => return AbiResponse::status(status),
        };
        if self.cfc.is_zero() || self.target.is_zero() || self.dimension.is_zero() {
            return AbiResponse::status(AbiStatus::Invalid);
        }
        if self
            .broker
            .authorize_in(
                self.cfc,
                request.handle_id,
                self.target,
                self.dimension,
                operation,
                tick,
            )
            .is_err()
            || self
                .broker
                .authorize_requester(
                    request.handle_id,
                    request.caller,
                    self.target,
                    self.dimension,
                    operation,
                    tick,
                )
                .is_err()
        {
            return AbiResponse::status(AbiStatus::Denied);
        }
        let Some(call) = AbiCall::from_raw(request.call) else {
            return AbiResponse::status(AbiStatus::Unsupported);
        };
        service(call, request.arguments)
    }
}

impl AbiResponse {
    pub const fn status(status: AbiStatus) -> Self {
        Self {
            status: status as u16,
            values: [0; 4],
        }
    }

    pub const fn ok(values: [u64; 4]) -> Self {
        Self {
            status: AbiStatus::Ok as u16,
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_abi_calls_are_versioned_and_capability_gated() {
        let request = AbiRequest {
            version: FORM_ABI_VERSION,
            call: AbiCall::SurfaceCommit as u16,
            caller: Fin::from_u128(7),
            handle_id: 3,
            arguments: [0; 6],
        };
        assert_eq!(request.validate(), Ok(Operations::DISPLAY));
        assert_eq!(
            AbiRequest {
                version: 2,
                ..request
            }
            .validate(),
            Err(AbiStatus::Invalid)
        );
        assert_eq!(
            AbiRequest {
                handle_id: 0,
                ..request
            }
            .validate(),
            Err(AbiStatus::Denied)
        );
        assert_eq!(core::mem::size_of::<AbiRequest>(), 72);
        assert_eq!(core::mem::size_of::<AbiResponse>(), 40);
    }

    #[test]
    fn form_abi_v1_call_numbers_and_rights_are_frozen() {
        let caller = Fin::from_u128(7);
        let operation = |call| {
            AbiRequest {
                version: FORM_ABI_VERSION,
                call: call as u16,
                caller,
                handle_id: 1,
                arguments: [0; 6],
            }
            .validate()
        };

        assert_eq!(AbiCall::TimeNow as u16, 4);
        assert_eq!(AbiCall::SurfaceCreate as u16, 16);
        assert_eq!(AbiCall::StorageRead as u16, 33);
        assert_eq!(AbiCall::NetworkReceive as u16, 36);
        assert_eq!(AbiCall::PackageTransaction as u16, 48);
        assert_eq!(operation(AbiCall::IpcSend), Ok(Operations::EXECUTE));
        assert_eq!(operation(AbiCall::ExecutionYield), Ok(Operations::EXECUTE));
        assert_eq!(operation(AbiCall::ExecutionExit), Ok(Operations::EXECUTE));
        assert_eq!(operation(AbiCall::StorageWrite), Ok(Operations::CONFIGURE));
        assert_eq!(operation(AbiCall::NetworkSend), Ok(Operations::NETWORK));
        assert_eq!(
            AbiRequest {
                version: FORM_ABI_VERSION,
                call: 255,
                caller,
                handle_id: 1,
                arguments: [0; 6],
            }
            .validate(),
            Err(AbiStatus::Unsupported)
        );
    }

    #[test]
    fn native_call_gate_requires_the_callers_cfc_bound_handle() {
        let cfc = CfcFin::from_u128(1);
        let caller = Fin::from_u128(2);
        let target = Fin::from_u128(3);
        let dimension = Fin::from_u128(4);
        let mut broker = CapabilityBroker::new(cfc);
        let handle = broker
            .issue_for(
                caller,
                crate::Authority::Operator,
                target,
                dimension,
                Operations::PACKAGE,
                100,
            )
            .unwrap();
        let request = AbiRequest {
            version: FORM_ABI_VERSION,
            call: AbiCall::PackageTransaction as u16,
            caller,
            handle_id: handle.id,
            arguments: [7, 9, 0, 0, 0, 0],
        };
        let response = NativeCallGate::new(&broker, cfc, target, dimension).dispatch(
            request,
            1,
            |call, arguments| {
                assert_eq!(call, AbiCall::PackageTransaction);
                assert_eq!(&arguments[..2], &[7, 9]);
                AbiResponse::ok([42, 0, 0, 0])
            },
        );
        assert_eq!(AbiStatus::from_raw(response.status), Some(AbiStatus::Ok));
        assert_eq!(response.values[0], 42);

        let denied = NativeCallGate::new(&broker, CfcFin::from_u128(99), target, dimension)
            .dispatch(request, 1, |_, _| panic!("foreign CFC reached service"));
        assert_eq!(AbiStatus::from_raw(denied.status), Some(AbiStatus::Denied));
    }
}
