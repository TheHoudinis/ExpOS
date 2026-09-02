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
    pub const DISPLAY: Self = Self(1 << 6);
    pub const INPUT: Self = Self(1 << 7);

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
    pub parent_id: u32,
    pub requester: Fin,
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
    Amplification,
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
        self.issue_for(
            Fin::ZERO,
            authority,
            target,
            dimension,
            operations,
            valid_until_tick,
        )
    }

    pub fn issue_for(
        &mut self,
        requester: Fin,
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
            parent_id: 0,
            requester,
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

    /// Derive a narrower Handle without consulting ambient authority. A
    /// delegated Handle can never add operations, outlive its parent, change
    /// target/Dimension, or survive revocation of the parent chain.
    pub fn delegate(
        &mut self,
        parent_id: u32,
        requester: Fin,
        operations: Operations,
        valid_until_tick: u64,
        tick: u64,
    ) -> Result<FormHandle, CapabilityError> {
        let parent = self
            .handles
            .iter()
            .flatten()
            .find(|handle| handle.id == parent_id)
            .copied()
            .ok_or(CapabilityError::NotFound)?;
        self.authorize(parent.id, parent.target, parent.dimension, operations, tick)?;
        if requester.is_zero()
            || operations == Operations::NONE
            || !parent.operations.contains(operations)
            || valid_until_tick > parent.valid_until_tick
        {
            return Err(CapabilityError::Amplification);
        }
        let slot = self
            .handles
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CapabilityError::Full)?;
        let handle = FormHandle {
            id: self.next_id,
            parent_id: parent.id,
            requester,
            target: parent.target,
            dimension: parent.dimension,
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

    pub fn authorize_requester(
        &self,
        id: u32,
        requester: Fin,
        target: Fin,
        dimension: Fin,
        operation: Operations,
        tick: u64,
    ) -> Result<(), CapabilityError> {
        self.authorize(id, target, dimension, operation, tick)?;
        let handle = self
            .handles
            .iter()
            .flatten()
            .find(|handle| handle.id == id)
            .ok_or(CapabilityError::NotFound)?;
        if handle.requester != requester {
            return Err(CapabilityError::Denied);
        }
        Ok(())
    }

    pub fn revoke(&mut self, id: u32) -> Result<(), CapabilityError> {
        if !self.handles.iter().flatten().any(|handle| handle.id == id) {
            return Err(CapabilityError::NotFound);
        }
        let mut changed = true;
        while changed {
            changed = false;
            let snapshot = self.handles;
            for (index, handle) in snapshot.iter().enumerate() {
                let Some(handle) = handle else { continue };
                let parent_revoked = handle.parent_id != 0
                    && snapshot
                        .iter()
                        .flatten()
                        .any(|parent| parent.id == handle.parent_id && parent.revoked);
                if (handle.id == id || parent_revoked) && !handle.revoked {
                    if let Some(stored) = self.handles[index].as_mut() {
                        stored.revoked = true;
                        changed = true;
                    }
                }
            }
        }
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

    #[test]
    fn requester_identity_is_part_of_authorization() {
        let requester = Fin::from_u128(10);
        let target = Fin::from_u128(11);
        let dimension = Fin::from_u128(12);
        let mut broker = CapabilityBroker::new();
        let handle = broker
            .issue_for(
                requester,
                Authority::Operator,
                target,
                dimension,
                Operations::PACKAGE,
                20,
            )
            .unwrap();
        assert!(broker
            .authorize_requester(
                handle.id,
                requester,
                target,
                dimension,
                Operations::PACKAGE,
                1
            )
            .is_ok());
        assert_eq!(
            broker.authorize_requester(
                handle.id,
                Fin::from_u128(99),
                target,
                dimension,
                Operations::PACKAGE,
                1
            ),
            Err(CapabilityError::Denied)
        );
    }

    #[test]
    fn delegation_is_narrow_and_parent_revocation_cascades() {
        let display = Fin::from_u128(21);
        let stable = Fin::from_u128(22);
        let browser = Fin::from_u128(23);
        let mut broker = CapabilityBroker::new();
        let root = broker
            .issue_for(
                Fin::from_u128(20),
                Authority::Operator,
                display,
                stable,
                Operations::DISPLAY.union(Operations::INPUT),
                100,
            )
            .unwrap();
        let child = broker
            .delegate(root.id, browser, Operations::DISPLAY, 80, 1)
            .unwrap();
        assert_eq!(child.parent_id, root.id);
        assert_eq!(
            broker.delegate(
                child.id,
                Fin::from_u128(24),
                Operations::DISPLAY.union(Operations::INPUT),
                80,
                1,
            ),
            Err(CapabilityError::OperationDenied)
        );
        broker.revoke(root.id).unwrap();
        assert_eq!(
            broker.authorize_requester(child.id, browser, display, stable, Operations::DISPLAY, 2,),
            Err(CapabilityError::Revoked)
        );
    }
}
