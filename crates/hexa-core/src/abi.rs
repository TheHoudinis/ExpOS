use crate::{Fin, Operations};

pub const GO_ABI_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum AbiCall {
    Log = 1,
    FormResolve = 2,
    HandleAuthorize = 3,
    SurfaceCreate = 16,
    BufferAttach = 17,
    SurfaceDamage = 18,
    SurfaceCommit = 19,
    EventPoll = 20,
    BrowserNavigate = 32,
    PackageTransaction = 48,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AbiRequest {
    pub version: u16,
    pub call: AbiCall,
    pub caller: Fin,
    pub handle_id: u32,
    pub arguments: [u64; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AbiResponse {
    pub status: AbiStatus,
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

impl AbiRequest {
    pub fn validate(self) -> Result<Operations, AbiStatus> {
        if self.version != GO_ABI_VERSION || self.caller.is_zero() {
            return Err(AbiStatus::Invalid);
        }
        let operation = match self.call {
            AbiCall::Log | AbiCall::FormResolve | AbiCall::EventPoll => Operations::READ,
            AbiCall::HandleAuthorize => Operations::EXECUTE,
            AbiCall::SurfaceCreate
            | AbiCall::BufferAttach
            | AbiCall::SurfaceDamage
            | AbiCall::SurfaceCommit
            | AbiCall::BrowserNavigate => Operations::EXECUTE,
            AbiCall::PackageTransaction => Operations::PACKAGE,
        };
        if self.handle_id == 0 && !matches!(self.call, AbiCall::Log) {
            return Err(AbiStatus::Denied);
        }
        Ok(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_abi_calls_are_versioned_and_capability_gated() {
        let request = AbiRequest {
            version: GO_ABI_VERSION,
            call: AbiCall::SurfaceCommit,
            caller: Fin::from_u128(7),
            handle_id: 3,
            arguments: [0; 6],
        };
        assert_eq!(request.validate(), Ok(Operations::EXECUTE));
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
}
