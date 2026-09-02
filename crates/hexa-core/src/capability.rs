use crate::Fin;

const MAX_HANDLES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Operator,
    Power,
    Guest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operations(u16);

impl Operations {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const EXECUTE: Self = Self(1 << 1);
    pub const CONFIGURE: Self = Self(1 << 2);
    pub const RELATE: Self = Self(1 << 3);
    pub const RETIRE: Self = Self(1 << 4);
    pub const PACKAGE: Self = Self(1 << 5);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormHandle {
    pub id: u32,
    pub target: Fin,
    pub dimension: Fin,
    pub operations: Operations,
    pub valid_until_tick: u64,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityError {
    Denied,
    Full,
    NotFound,
    Revoked,
    Expired,
    WrongDimension,
    OperationDenied,
}

pub struct CapabilityBroker {
    handles: [Option<FormHandle>; MAX_HANDLES],
    next_id: u32,
}

impl CapabilityBroker {
    pub const fn new() -> Self {
        Self {
            handles: [None; MAX_HANDLES],
            next_id: 1,
        }
    }

    pub fn issue(
        &mut self,
        authority: Authority,
        target: Fin,
        dimension: Fin,
        operations: Operations,
        valid_until_tick: u64,
    ) -> Result<FormHandle, CapabilityError> {
        let permitted = match authority {
            Authority::Operator => true,
            Authority::Power => {
                !operations.contains(Operations::RETIRE)
                    && !operations.contains(Operations::CONFIGURE)
            }
            Authority::Guest => operations.bits() != 0 && Operations::READ.contains(operations),
        };
        if !permitted || operations == Operations::NONE || target.is_zero() || dimension.is_zero() {
            return Err(CapabilityError::Denied);
        }
        let slot = self
            .handles
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CapabilityError::Full)?;
        let handle = FormHandle {
            id: self.next_id,
            target,
            dimension,
            operations,
            valid_until_tick,
            revoked: false,
        };
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *slot = Some(handle);
        Ok(handle)
    }

    pub fn authorize(
        &self,
        id: u32,
        target: Fin,
        dimension: Fin,
        operation: Operations,
        tick: u64,
    ) -> Result<(), CapabilityError> {
        let handle = self
            .handles
            .iter()
            .flatten()
            .find(|handle| handle.id == id)
            .ok_or(CapabilityError::NotFound)?;
        if handle.revoked {
            return Err(CapabilityError::Revoked);
        }
        if tick > handle.valid_until_tick {
            return Err(CapabilityError::Expired);
        }
        if handle.target != target || handle.dimension != dimension {
            return Err(CapabilityError::WrongDimension);
        }
        if !handle.operations.contains(operation) {
            return Err(CapabilityError::OperationDenied);
        }
        Ok(())
    }

    pub fn revoke(&mut self, id: u32) -> Result<(), CapabilityError> {
        let handle = self
            .handles
            .iter_mut()
            .flatten()
            .find(|handle| handle.id == id)
            .ok_or(CapabilityError::NotFound)?;
        handle.revoked = true;
        Ok(())
    }
}

impl Default for CapabilityBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_are_scoped_and_revocable() {
        let target = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        let mut broker = CapabilityBroker::new();
        let handle = broker
            .issue(Authority::Power, target, dimension, Operations::READ, 10)
            .unwrap();
        assert!(broker
            .authorize(handle.id, target, dimension, Operations::READ, 9)
            .is_ok());
        assert_eq!(
            broker.authorize(handle.id, target, Fin::from_u128(3), Operations::READ, 9),
            Err(CapabilityError::WrongDimension)
        );
        broker.revoke(handle.id).unwrap();
        assert_eq!(
            broker.authorize(handle.id, target, dimension, Operations::READ, 9),
            Err(CapabilityError::Revoked)
        );
    }
}
