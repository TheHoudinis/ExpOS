use crate::{CfcFin, Fin};

const MAX_HANDLES: usize = 32;
const MAX_SEALS: usize = 16;

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
    pub const NETWORK: Self = Self(1 << 8);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !0x01FF == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormHandle {
    pub id: u32,
    pub parent_id: u32,
    /// The CFC is part of capability identity. A Handle can never be moved to
    /// another CFC, even when the target and Dimension FINs are otherwise
    /// well-formed.
    pub cfc: CfcFin,
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
    WrongCfc,
    OperationDenied,
    Amplification,
    Sealed,
    AlreadySealed,
    SealTableFull,
    Duplicate,
}

/// A monotonic ceiling over ambient capability issuance for one execution
/// context during the lifetime of its [`CapabilityBroker`]. ExpSeal does not
/// replace Handle revocation: it prevents new root authority from appearing
/// after activation while existing Handles may only be reduced through normal
/// delegation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpSeal {
    pub cfc: CfcFin,
    pub requester: Fin,
    pub dimension: Fin,
    pub root_handle_limit: u32,
}

pub struct CapabilityBroker {
    cfc: CfcFin,
    handles: [Option<FormHandle>; MAX_HANDLES],
    seals: [Option<ExpSeal>; MAX_SEALS],
    next_id: u32,
}

impl CapabilityBroker {
    pub const fn new(cfc: CfcFin) -> Self {
        assert!(!cfc.is_zero(), "a capability broker must belong to a CFC");
        Self {
            cfc,
            handles: [None; MAX_HANDLES],
            seals: [None; MAX_SEALS],
            next_id: 1,
        }
    }

    pub const fn cfc(&self) -> CfcFin {
        self.cfc
    }

    /// Restore a Handle from an authenticated/persisted capability record.
    /// Runtime authority is not consulted because this is not ambient
    /// issuance; structural validity and the complete parent chain are still
    /// enforced before publication.
    pub fn restore(&mut self, handle: FormHandle) -> Result<(), CapabilityError> {
        if handle.cfc != self.cfc {
            return Err(CapabilityError::WrongCfc);
        }
        if handle.id == 0
            || handle.requester.is_zero()
            || handle.target.is_zero()
            || handle.dimension.is_zero()
            || handle.operations == Operations::NONE
        {
            return Err(CapabilityError::Denied);
        }
        if self
            .handles
            .iter()
            .flatten()
            .any(|entry| entry.id == handle.id)
        {
            return Err(CapabilityError::Duplicate);
        }
        if handle.parent_id != 0
            && !self
                .handles
                .iter()
                .flatten()
                .any(|entry| entry.id == handle.parent_id)
        {
            return Err(CapabilityError::NotFound);
        }
        let slot = self
            .handles
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CapabilityError::Full)?;
        *slot = Some(handle);
        self.next_id = self.next_id.max(handle.id.wrapping_add(1).max(1));
        Ok(())
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
        if self.is_sealed(requester, dimension) {
            return Err(CapabilityError::Sealed);
        }
        let permitted = match authority {
            Authority::Operator => true,
            Authority::Power => {
                !operations.contains(Operations::RETIRE)
                    && !operations.contains(Operations::CONFIGURE)
            }
            Authority::Guest => operations.bits() != 0 && Operations::READ.contains(operations),
        };
        if !permitted
            || requester.is_zero()
            || operations == Operations::NONE
            || target.is_zero()
            || dimension.is_zero()
        {
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
            cfc: self.cfc,
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

    /// Close ambient/root Handle issuance for a requester in one Dimension for
    /// this broker's lifetime. There is deliberately no unseal operation.
    /// Handles issued before the seal remain usable and may derive only
    /// strictly narrower children.
    pub fn seal_context(
        &mut self,
        requester: Fin,
        dimension: Fin,
    ) -> Result<ExpSeal, CapabilityError> {
        if requester.is_zero() || dimension.is_zero() {
            return Err(CapabilityError::Denied);
        }
        if self.is_sealed(requester, dimension) {
            return Err(CapabilityError::AlreadySealed);
        }
        let slot = self
            .seals
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CapabilityError::SealTableFull)?;
        let seal = ExpSeal {
            cfc: self.cfc,
            requester,
            dimension,
            root_handle_limit: self.next_id.saturating_sub(1),
        };
        *slot = Some(seal);
        Ok(seal)
    }

    pub fn seal(&self, requester: Fin, dimension: Fin) -> Option<ExpSeal> {
        self.seals
            .iter()
            .flatten()
            .find(|seal| seal.requester == requester && seal.dimension == dimension)
            .copied()
    }

    fn is_sealed(&self, requester: Fin, dimension: Fin) -> bool {
        self.seal(requester, dimension).is_some()
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
        if self.is_sealed(requester, parent.dimension) && requester != parent.requester {
            return Err(CapabilityError::Sealed);
        }
        self.authorize(parent.id, parent.target, parent.dimension, operations, tick)?;
        if requester.is_zero()
            || operations == Operations::NONE
            || !parent.operations.contains(operations)
            || valid_until_tick > parent.valid_until_tick
            || (operations == parent.operations && valid_until_tick == parent.valid_until_tick)
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
            cfc: self.cfc,
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

    pub fn authorize_in(
        &self,
        cfc: CfcFin,
        id: u32,
        target: Fin,
        dimension: Fin,
        operation: Operations,
        tick: u64,
    ) -> Result<(), CapabilityError> {
        if cfc != self.cfc {
            return Err(CapabilityError::WrongCfc);
        }
        let handle = self
            .handles
            .iter()
            .flatten()
            .find(|handle| handle.id == id)
            .ok_or(CapabilityError::NotFound)?;
        if handle.cfc != cfc {
            return Err(CapabilityError::WrongCfc);
        }
        self.authorize(id, target, dimension, operation, tick)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(value: u128) -> Fin {
        Fin::from_u128(value)
    }

    fn cfc_fin(value: u128) -> CfcFin {
        CfcFin::from_u128(value)
    }

    #[test]
    fn handles_are_scoped_and_revocable() {
        let target = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        let mut broker = CapabilityBroker::new(cfc_fin(100));
        let handle = broker
            .issue_for(
                fin(9),
                Authority::Power,
                target,
                dimension,
                Operations::READ,
                10,
            )
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
        let mut broker = CapabilityBroker::new(cfc_fin(100));
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
    fn network_handles_require_explicit_non_guest_authority() {
        let requester = Fin::from_u128(30);
        let network = Fin::from_u128(31);
        let stable = Fin::from_u128(32);
        let mut broker = CapabilityBroker::new(cfc_fin(100));
        assert_eq!(
            broker.issue_for(
                requester,
                Authority::Guest,
                network,
                stable,
                Operations::NETWORK,
                100,
            ),
            Err(CapabilityError::Denied)
        );
        let handle = broker
            .issue_for(
                requester,
                Authority::Power,
                network,
                stable,
                Operations::NETWORK,
                100,
            )
            .unwrap();
        assert!(broker
            .authorize_requester(
                handle.id,
                requester,
                network,
                stable,
                Operations::NETWORK,
                1,
            )
            .is_ok());
    }

    #[test]
    fn delegation_is_narrow_and_parent_revocation_cascades() {
        let display = Fin::from_u128(21);
        let stable = Fin::from_u128(22);
        let browser = Fin::from_u128(23);
        let mut broker = CapabilityBroker::new(cfc_fin(100));
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

    #[test]
    fn cfc_is_part_of_authorization_identity() {
        let cfc = cfc_fin(100);
        let other_cfc = cfc_fin(101);
        let target = fin(1);
        let dimension = fin(2);
        let mut broker = CapabilityBroker::new(cfc);
        let handle = broker
            .issue_for(
                fin(9),
                Authority::Operator,
                target,
                dimension,
                Operations::READ,
                10,
            )
            .unwrap();
        assert_eq!(handle.cfc, cfc);
        assert!(broker
            .authorize_in(cfc, handle.id, target, dimension, Operations::READ, 1)
            .is_ok());
        assert_eq!(
            broker.authorize_in(other_cfc, handle.id, target, dimension, Operations::READ, 1,),
            Err(CapabilityError::WrongCfc)
        );
    }

    #[test]
    fn expseal_closes_root_issuance_but_preserves_attenuation() {
        let cfc = cfc_fin(100);
        let requester = fin(10);
        let target = fin(11);
        let dimension = fin(12);
        let mut broker = CapabilityBroker::new(cfc);
        let root = broker
            .issue_for(
                requester,
                Authority::Operator,
                target,
                dimension,
                Operations::READ.union(Operations::EXECUTE),
                20,
            )
            .unwrap();
        let seal = broker.seal_context(requester, dimension).unwrap();
        assert_eq!(seal.cfc, cfc);
        assert_eq!(seal.root_handle_limit, root.id);
        assert_eq!(
            broker.issue_for(
                requester,
                Authority::Operator,
                target,
                dimension,
                Operations::READ,
                20,
            ),
            Err(CapabilityError::Sealed)
        );
        let child = broker
            .delegate(root.id, requester, Operations::READ, 10, 1)
            .unwrap();
        assert_eq!(child.parent_id, root.id);
        assert_eq!(
            broker.delegate(root.id, requester, Operations::CONFIGURE, 10, 1),
            Err(CapabilityError::OperationDenied)
        );
        assert_eq!(
            broker.seal_context(requester, dimension),
            Err(CapabilityError::AlreadySealed)
        );
    }

    #[test]
    fn zero_requesters_and_exact_delegation_clones_are_rejected() {
        let mut broker = CapabilityBroker::new(cfc_fin(100));
        let requester = fin(10);
        let target = fin(11);
        let dimension = fin(12);
        assert_eq!(
            broker.issue_for(
                Fin::ZERO,
                Authority::Operator,
                target,
                dimension,
                Operations::READ,
                20,
            ),
            Err(CapabilityError::Denied)
        );

        let root = broker
            .issue_for(
                requester,
                Authority::Operator,
                target,
                dimension,
                Operations::READ.union(Operations::EXECUTE),
                20,
            )
            .unwrap();
        assert_eq!(
            broker.delegate(root.id, fin(13), root.operations, root.valid_until_tick, 1,),
            Err(CapabilityError::Amplification)
        );
        assert!(broker
            .delegate(root.id, fin(13), root.operations, 19, 1)
            .is_ok());
    }

    #[test]
    fn persisted_handles_restore_without_reissuing_ambient_authority() {
        let cfc = cfc_fin(100);
        let handle = FormHandle {
            id: 7,
            parent_id: 0,
            cfc,
            requester: fin(10),
            target: fin(11),
            dimension: fin(12),
            operations: Operations::EXECUTE,
            valid_until_tick: 50,
            revoked: false,
        };
        let mut broker = CapabilityBroker::new(cfc);
        broker.restore(handle).unwrap();
        assert!(broker
            .authorize(
                handle.id,
                handle.target,
                handle.dimension,
                Operations::EXECUTE,
                1
            )
            .is_ok());
        assert_eq!(broker.restore(handle), Err(CapabilityError::Duplicate));
        let next = broker
            .issue_for(
                fin(10),
                Authority::Operator,
                fin(13),
                fin(12),
                Operations::READ,
                50,
            )
            .unwrap();
        assert_eq!(next.id, 8);
    }
}
