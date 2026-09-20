//! Fixed-capacity resource budgets for one execution context.
//!
//! A caller charges a resource before beginning work and releases resources
//! that are no longer held. Charges are atomic: a rejected charge never
//! changes `used`, while the rejection is retained in the resource account's
//! counters for diagnostics.

/// The resources that ExpOS can account to an execution context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResourceKind {
    ExecutionCheckpoints = 0,
    HeapPages = 1,
    IpcMessages = 2,
    IpcBytes = 3,
    OutputBytes = 4,
    NetworkBytes = 5,
    FormOperations = 6,
    FormBytes = 7,
}

pub const RESOURCE_KIND_COUNT: usize = 8;

impl ResourceKind {
    pub const ALL: [Self; RESOURCE_KIND_COUNT] = [
        Self::ExecutionCheckpoints,
        Self::HeapPages,
        Self::IpcMessages,
        Self::IpcBytes,
        Self::OutputBytes,
        Self::NetworkBytes,
        Self::FormOperations,
        Self::FormBytes,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// A read-only view of one resource's limits, current use, and rejected work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceAccount {
    ceiling: u64,
    soft: u64,
    hard: u64,
    used: u64,
    denied_charges: u64,
    overflow_charges: u64,
}

impl ResourceAccount {
    const ZERO: Self = Self {
        ceiling: 0,
        soft: 0,
        hard: 0,
        used: 0,
        denied_charges: 0,
        overflow_charges: 0,
    };

    const fn with_ceiling(ceiling: u64) -> Self {
        Self {
            ceiling,
            soft: ceiling,
            hard: ceiling,
            used: 0,
            denied_charges: 0,
            overflow_charges: 0,
        }
    }

    pub const fn ceiling(self) -> u64 {
        self.ceiling
    }

    pub const fn soft(self) -> u64 {
        self.soft
    }

    pub const fn hard(self) -> u64 {
        self.hard
    }

    pub const fn used(self) -> u64 {
        self.used
    }

    pub const fn denied_charges(self) -> u64 {
        self.denied_charges
    }

    pub const fn overflow_charges(self) -> u64 {
        self.overflow_charges
    }

    pub const fn remaining_before_soft(self) -> u64 {
        self.soft - self.used
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    InvalidLimits {
        kind: ResourceKind,
        used: u64,
        soft: u64,
        hard: u64,
        ceiling: u64,
    },
    SoftLimitExceeded {
        kind: ResourceKind,
        used: u64,
        requested: u64,
        soft: u64,
    },
    ChargeOverflow {
        kind: ResourceKind,
        used: u64,
        requested: u64,
    },
    ReleaseUnderflow {
        kind: ResourceKind,
        used: u64,
        requested: u64,
    },
}

/// A fixed-resource budget owned by one execution context.
///
/// `C` is the caller's context identity (for example a scheduler context ID or
/// a tuple of CFC/Dimension/Form FINs). It is stored verbatim and never used as
/// ambient authority. The ceiling array follows [`ResourceKind::ALL`] order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpBudget<C> {
    context: C,
    accounts: [ResourceAccount; RESOURCE_KIND_COUNT],
}

impl<C> ExpBudget<C> {
    pub fn new(context: C, ceilings: [u64; RESOURCE_KIND_COUNT]) -> Self {
        let mut accounts = [ResourceAccount::ZERO; RESOURCE_KIND_COUNT];
        let mut index = 0;
        while index < RESOURCE_KIND_COUNT {
            accounts[index] = ResourceAccount::with_ceiling(ceilings[index]);
            index += 1;
        }
        Self { context, accounts }
    }

    /// Construct a budget that uses the same immutable ceiling for every
    /// resource kind.
    pub fn uniform(context: C, ceiling: u64) -> Self {
        Self::new(context, [ceiling; RESOURCE_KIND_COUNT])
    }

    pub const fn context(&self) -> &C {
        &self.context
    }

    pub fn account(&self, kind: ResourceKind) -> ResourceAccount {
        self.accounts[kind.index()]
    }

    /// Change the enforced soft limit and its administrative hard bound
    /// without changing the permanent ceiling. Limits cannot be moved below
    /// current use.
    pub fn configure(
        &mut self,
        kind: ResourceKind,
        soft: u64,
        hard: u64,
    ) -> Result<(), BudgetError> {
        let account = &mut self.accounts[kind.index()];
        if account.used > soft || soft > hard || hard > account.ceiling {
            return Err(BudgetError::InvalidLimits {
                kind,
                used: account.used,
                soft,
                hard,
                ceiling: account.ceiling,
            });
        }
        account.soft = soft;
        account.hard = hard;
        Ok(())
    }

    /// Reserve resource units before work begins.
    ///
    /// A soft-limit denial or arithmetic overflow leaves `used` unchanged and
    /// increments `denied_charges`. Arithmetic overflow also increments the
    /// more specific `overflow_charges` counter. Diagnostic counters saturate
    /// rather than wrapping.
    pub fn charge(&mut self, kind: ResourceKind, amount: u64) -> Result<u64, BudgetError> {
        let account = &mut self.accounts[kind.index()];
        let Some(charged) = account.used.checked_add(amount) else {
            account.denied_charges = account.denied_charges.saturating_add(1);
            account.overflow_charges = account.overflow_charges.saturating_add(1);
            return Err(BudgetError::ChargeOverflow {
                kind,
                used: account.used,
                requested: amount,
            });
        };
        if charged > account.soft {
            account.denied_charges = account.denied_charges.saturating_add(1);
            return Err(BudgetError::SoftLimitExceeded {
                kind,
                used: account.used,
                requested: amount,
                soft: account.soft,
            });
        }
        account.used = charged;
        Ok(charged)
    }

    /// Release exactly `amount` units. An over-release is rejected atomically
    /// instead of silently clamping usage to zero.
    pub fn release(&mut self, kind: ResourceKind, amount: u64) -> Result<(), BudgetError> {
        let account = &mut self.accounts[kind.index()];
        let Some(remaining) = account.used.checked_sub(amount) else {
            return Err(BudgetError::ReleaseUnderflow {
                kind,
                used: account.used,
                requested: amount,
            });
        };
        account.used = remaining;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ceilings() -> [u64; RESOURCE_KIND_COUNT] {
        [100, 20, 10, 1_000, 500, 2_000, 40, 800]
    }

    #[test]
    fn budget_is_keyed_by_its_execution_context() {
        let budget = ExpBudget::new((11_u32, 7_u32), ceilings());
        assert_eq!(budget.context(), &(11, 7));
        assert_eq!(
            budget.account(ResourceKind::ExecutionCheckpoints).ceiling(),
            100
        );
        assert_eq!(budget.account(ResourceKind::FormBytes).ceiling(), 800);
    }

    #[test]
    fn configuration_preserves_order_and_the_immutable_ceiling() {
        let mut budget = ExpBudget::new(1_u32, ceilings());
        budget.configure(ResourceKind::HeapPages, 8, 12).unwrap();
        let account = budget.account(ResourceKind::HeapPages);
        assert_eq!(
            (account.soft(), account.hard(), account.ceiling()),
            (8, 12, 20)
        );

        assert_eq!(
            budget.configure(ResourceKind::HeapPages, 13, 12),
            Err(BudgetError::InvalidLimits {
                kind: ResourceKind::HeapPages,
                used: 0,
                soft: 13,
                hard: 12,
                ceiling: 20,
            })
        );
        assert_eq!(budget.account(ResourceKind::HeapPages), account);

        assert!(matches!(
            budget.configure(ResourceKind::HeapPages, 12, 21),
            Err(BudgetError::InvalidLimits { .. })
        ));
        assert_eq!(budget.account(ResourceKind::HeapPages).ceiling(), 20);
    }

    #[test]
    fn the_soft_limit_is_the_runtime_enforcement_boundary() {
        let mut budget = ExpBudget::uniform("worker", 20);
        budget.configure(ResourceKind::IpcMessages, 5, 10).unwrap();
        assert_eq!(budget.charge(ResourceKind::IpcMessages, 5), Ok(5));
        assert_eq!(
            budget.charge(ResourceKind::IpcMessages, 1),
            Err(BudgetError::SoftLimitExceeded {
                kind: ResourceKind::IpcMessages,
                used: 5,
                requested: 1,
                soft: 5,
            })
        );
        let account = budget.account(ResourceKind::IpcMessages);
        assert_eq!(account.used(), 5);
        assert_eq!(account.remaining_before_soft(), 0);
        assert_eq!(account.hard(), 10);
    }

    #[test]
    fn soft_limit_denial_is_atomic_and_accounted() {
        let mut budget = ExpBudget::uniform(9_u8, 100);
        budget.configure(ResourceKind::OutputBytes, 6, 8).unwrap();
        budget.charge(ResourceKind::OutputBytes, 6).unwrap();
        assert_eq!(
            budget.charge(ResourceKind::OutputBytes, 2),
            Err(BudgetError::SoftLimitExceeded {
                kind: ResourceKind::OutputBytes,
                used: 6,
                requested: 2,
                soft: 6,
            })
        );
        let account = budget.account(ResourceKind::OutputBytes);
        assert_eq!(account.used(), 6);
        assert_eq!(account.denied_charges(), 1);
        assert_eq!(account.overflow_charges(), 0);
    }

    #[test]
    fn arithmetic_overflow_is_denied_and_counted_separately() {
        let mut budget = ExpBudget::uniform(9_u8, u64::MAX);
        budget.charge(ResourceKind::NetworkBytes, u64::MAX).unwrap();
        assert!(matches!(
            budget.charge(ResourceKind::NetworkBytes, 1),
            Err(BudgetError::ChargeOverflow { .. })
        ));
        let account = budget.account(ResourceKind::NetworkBytes);
        assert_eq!(account.used(), u64::MAX);
        assert_eq!(account.denied_charges(), 1);
        assert_eq!(account.overflow_charges(), 1);
    }

    #[test]
    fn release_is_exact_and_underflow_does_not_mutate_usage() {
        let mut budget = ExpBudget::uniform(9_u8, 100);
        budget.charge(ResourceKind::FormOperations, 10).unwrap();
        budget.release(ResourceKind::FormOperations, 4).unwrap();
        assert_eq!(budget.account(ResourceKind::FormOperations).used(), 6);
        assert_eq!(
            budget.release(ResourceKind::FormOperations, 7),
            Err(BudgetError::ReleaseUnderflow {
                kind: ResourceKind::FormOperations,
                used: 6,
                requested: 7,
            })
        );
        assert_eq!(budget.account(ResourceKind::FormOperations).used(), 6);
    }

    #[test]
    fn limits_cannot_be_reconfigured_below_current_use() {
        let mut budget = ExpBudget::uniform(9_u8, 100);
        budget.charge(ResourceKind::FormBytes, 30).unwrap();
        assert!(matches!(
            budget.configure(ResourceKind::FormBytes, 29, 40),
            Err(BudgetError::InvalidLimits { used: 30, .. })
        ));
        let account = budget.account(ResourceKind::FormBytes);
        assert_eq!(
            (account.soft(), account.hard(), account.ceiling()),
            (100, 100, 100)
        );
    }
}
