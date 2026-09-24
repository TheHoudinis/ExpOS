//! Form-native execution contexts and cooperative scheduling semantics.
//!
//! This module intentionally models what the scheduler owns without importing
//! PID/UID/file-descriptor concepts. Platform code remains responsible for
//! switching page tables and CPU registers at the dispatch boundary.

use crate::{CfcFin, ExpBudget, Fin, FormHandle, ResourceKind};

pub const MAX_EXECUTION_CONTEXTS: usize = 8;
pub const MAX_CONTEXT_HANDLES: usize = 16;
pub const MAX_CONTEXT_EVENTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionIdentity {
    pub cfc: CfcFin,
    pub dimension: Fin,
    pub form: Fin,
}

impl ExecutionIdentity {
    pub const fn new(cfc: CfcFin, dimension: Fin, form: Fin) -> Self {
        Self {
            cfc,
            dimension,
            form,
        }
    }

    pub const fn is_valid(self) -> bool {
        !self.cfc.is_zero() && !self.dimension.is_zero() && !self.form.is_zero()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressSpace {
    /// Architecture-owned page-table root or equivalent address-space token.
    pub root: u64,
    pub lower_bound: u64,
    pub upper_bound: u64,
}

impl AddressSpace {
    pub const fn kernel_bootstrap() -> Self {
        Self {
            root: 0,
            lower_bound: 0,
            upper_bound: u64::MAX,
        }
    }

    pub const fn confined(root: u64, lower_bound: u64, upper_bound: u64) -> Option<Self> {
        if root == 0 || lower_bound >= upper_bound {
            return None;
        }
        Some(Self {
            root,
            lower_bound,
            upper_bound,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuState {
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
    pub accumulator: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionEvent {
    pub source: Fin,
    pub code: u32,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionState {
    Ready,
    Running,
    Waiting,
    BudgetBlocked,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionRuntime {
    /// Entrypoint is implemented by a trusted native kernel subsystem.
    KernelNative,
    /// Content is executed by the bounded in-kernel ExpPython runtime.
    ExpPython,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    InvalidIdentity,
    InvalidAddressSpace,
    DuplicateContext,
    ContextTableFull,
    UnknownContext,
    HandleSetFull,
    DuplicateHandle,
    ForeignHandle,
    EventQueueFull,
    BudgetDenied,
    NotRunnable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionContext {
    id: u32,
    identity: ExecutionIdentity,
    address_space: AddressSpace,
    handles: [Option<FormHandle>; MAX_CONTEXT_HANDLES],
    events: [Option<ExecutionEvent>; MAX_CONTEXT_EVENTS],
    event_head: u8,
    event_len: u8,
    budget: ExpBudget<ExecutionIdentity>,
    cpu: CpuState,
    runtime: ExecutionRuntime,
    state: ExecutionState,
    dispatches: u64,
}

impl ExecutionContext {
    pub fn new(
        identity: ExecutionIdentity,
        address_space: AddressSpace,
        budget_ceiling: u64,
    ) -> Result<Self, ExecutionError> {
        if !identity.is_valid() {
            return Err(ExecutionError::InvalidIdentity);
        }
        if address_space.root != 0 && address_space.lower_bound >= address_space.upper_bound {
            return Err(ExecutionError::InvalidAddressSpace);
        }
        Ok(Self {
            id: 0,
            identity,
            address_space,
            handles: [None; MAX_CONTEXT_HANDLES],
            events: [None; MAX_CONTEXT_EVENTS],
            event_head: 0,
            event_len: 0,
            budget: ExpBudget::uniform(identity, budget_ceiling),
            cpu: CpuState::default(),
            runtime: ExecutionRuntime::KernelNative,
            state: ExecutionState::Ready,
            dispatches: 0,
        })
    }

    pub const fn id(&self) -> u32 {
        self.id
    }

    pub const fn identity(&self) -> ExecutionIdentity {
        self.identity
    }

    pub const fn address_space(&self) -> AddressSpace {
        self.address_space
    }

    pub const fn cpu_state(&self) -> CpuState {
        self.cpu
    }

    pub fn set_cpu_state(&mut self, cpu: CpuState) {
        self.cpu = cpu;
    }

    pub const fn runtime(&self) -> ExecutionRuntime {
        self.runtime
    }

    pub fn set_runtime(&mut self, runtime: ExecutionRuntime) {
        self.runtime = runtime;
    }

    pub const fn state(&self) -> ExecutionState {
        self.state
    }

    pub const fn dispatches(&self) -> u64 {
        self.dispatches
    }

    pub const fn budget(&self) -> &ExpBudget<ExecutionIdentity> {
        &self.budget
    }

    pub fn budget_mut(&mut self) -> &mut ExpBudget<ExecutionIdentity> {
        &mut self.budget
    }

    pub fn attach_handle(&mut self, handle: FormHandle) -> Result<(), ExecutionError> {
        if handle.cfc != self.identity.cfc
            || handle.dimension != self.identity.dimension
            || (handle.requester != self.identity.form && handle.target != self.identity.form)
        {
            return Err(ExecutionError::ForeignHandle);
        }
        if self
            .handles
            .iter()
            .flatten()
            .any(|item| item.id == handle.id)
        {
            return Err(ExecutionError::DuplicateHandle);
        }
        let slot = self
            .handles
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ExecutionError::HandleSetFull)?;
        *slot = Some(handle);
        Ok(())
    }

    pub fn handles(&self) -> impl Iterator<Item = FormHandle> + '_ {
        self.handles.iter().flatten().copied()
    }

    pub fn push_event(&mut self, event: ExecutionEvent) -> Result<(), ExecutionError> {
        if self.event_len as usize == MAX_CONTEXT_EVENTS {
            return Err(ExecutionError::EventQueueFull);
        }
        let slot = (self.event_head as usize + self.event_len as usize) % MAX_CONTEXT_EVENTS;
        self.events[slot] = Some(event);
        self.event_len += 1;
        Ok(())
    }

    pub fn pop_event(&mut self) -> Option<ExecutionEvent> {
        if self.event_len == 0 {
            return None;
        }
        let slot = self.event_head as usize;
        let event = self.events[slot].take();
        self.event_head = ((slot + 1) % MAX_CONTEXT_EVENTS) as u8;
        self.event_len -= 1;
        event
    }

    pub const fn pending_events(&self) -> usize {
        self.event_len as usize
    }
}

pub struct Scheduler {
    contexts: [Option<ExecutionContext>; MAX_EXECUTION_CONTEXTS],
    current_slot: Option<usize>,
    cursor: usize,
    next_id: u32,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            contexts: core::array::from_fn(|_| None),
            current_slot: None,
            cursor: 0,
            next_id: 1,
        }
    }

    pub fn admit(&mut self, mut context: ExecutionContext) -> Result<u32, ExecutionError> {
        if self
            .contexts
            .iter()
            .flatten()
            .any(|item| item.identity == context.identity && item.state != ExecutionState::Exited)
        {
            return Err(ExecutionError::DuplicateContext);
        }
        let slot = self
            .contexts
            .iter_mut()
            .find(|slot| {
                slot.as_ref()
                    .is_none_or(|context| context.state == ExecutionState::Exited)
            })
            .ok_or(ExecutionError::ContextTableFull)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        context.id = id;
        context.state = ExecutionState::Ready;
        *slot = Some(context);
        Ok(id)
    }

    pub fn context(&self, id: u32) -> Option<&ExecutionContext> {
        self.contexts
            .iter()
            .flatten()
            .find(|context| context.id == id)
    }

    pub fn context_mut(&mut self, id: u32) -> Option<&mut ExecutionContext> {
        self.contexts
            .iter_mut()
            .flatten()
            .find(|context| context.id == id)
    }

    pub fn current(&self) -> Option<&ExecutionContext> {
        self.current_slot
            .and_then(|slot| self.contexts[slot].as_ref())
    }

    pub fn visit(&self, mut visitor: impl FnMut(&ExecutionContext)) {
        for context in self.contexts.iter().flatten() {
            visitor(context);
        }
    }

    /// Select the next ready Form context and account one dispatch checkpoint.
    /// A context that exhausts its budget is moved to `BudgetBlocked` and the
    /// search continues without making it current.
    pub fn dispatch_next(&mut self) -> Result<u32, ExecutionError> {
        if let Some(slot) = self.current_slot.take() {
            if let Some(current) = self.contexts[slot].as_mut() {
                if current.state == ExecutionState::Running {
                    current.state = ExecutionState::Ready;
                }
            }
        }

        for offset in 0..MAX_EXECUTION_CONTEXTS {
            let slot = (self.cursor + offset) % MAX_EXECUTION_CONTEXTS;
            let Some(context) = self.contexts[slot].as_mut() else {
                continue;
            };
            if context.state != ExecutionState::Ready {
                continue;
            }
            if context
                .budget
                .charge(ResourceKind::ExecutionCheckpoints, 1)
                .is_err()
            {
                context.state = ExecutionState::BudgetBlocked;
                continue;
            }
            context.state = ExecutionState::Running;
            context.dispatches = context.dispatches.saturating_add(1);
            self.current_slot = Some(slot);
            self.cursor = (slot + 1) % MAX_EXECUTION_CONTEXTS;
            return Ok(context.id);
        }
        Err(ExecutionError::NotRunnable)
    }

    pub fn wait(&mut self, id: u32) -> Result<(), ExecutionError> {
        let context = self.context_mut(id).ok_or(ExecutionError::UnknownContext)?;
        context.state = ExecutionState::Waiting;
        if self.current().is_some_and(|current| current.id == id) {
            self.current_slot = None;
        }
        Ok(())
    }

    pub fn wake(&mut self, id: u32) -> Result<(), ExecutionError> {
        let context = self.context_mut(id).ok_or(ExecutionError::UnknownContext)?;
        if context.state != ExecutionState::Waiting {
            return Err(ExecutionError::NotRunnable);
        }
        context.state = ExecutionState::Ready;
        Ok(())
    }

    pub fn exit(&mut self, id: u32) -> Result<(), ExecutionError> {
        self.finish(id, 0)
    }

    /// Finish a dispatched Form and retain its bounded result in the modeled
    /// CPU accumulator for diagnostics. The context remains inspectable.
    pub fn finish(&mut self, id: u32, result: u64) -> Result<(), ExecutionError> {
        let context = self.context_mut(id).ok_or(ExecutionError::UnknownContext)?;
        context.cpu.accumulator = result;
        context.state = ExecutionState::Exited;
        if self.current().is_some_and(|current| current.id == id) {
            self.current_slot = None;
        }
        Ok(())
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Authority, Operations};

    fn identity(form: u128) -> ExecutionIdentity {
        ExecutionIdentity::new(
            CfcFin::from_u128(1),
            Fin::from_u128(2),
            Fin::from_u128(form),
        )
    }

    #[test]
    fn contexts_are_scheduled_by_form_identity_and_budget() {
        let mut scheduler = Scheduler::new();
        let first = scheduler
            .admit(
                ExecutionContext::new(identity(10), AddressSpace::kernel_bootstrap(), 2).unwrap(),
            )
            .unwrap();
        let second = scheduler
            .admit(
                ExecutionContext::new(identity(11), AddressSpace::kernel_bootstrap(), 2).unwrap(),
            )
            .unwrap();
        assert_eq!(scheduler.dispatch_next(), Ok(first));
        assert_eq!(scheduler.dispatch_next(), Ok(second));
        assert_eq!(scheduler.dispatch_next(), Ok(first));
        assert_eq!(scheduler.dispatch_next(), Ok(second));
        assert_eq!(scheduler.dispatch_next(), Err(ExecutionError::NotRunnable));
        assert_eq!(
            scheduler.context(first).unwrap().state(),
            ExecutionState::BudgetBlocked
        );
        assert_eq!(
            scheduler.context(second).unwrap().state(),
            ExecutionState::BudgetBlocked
        );
    }

    #[test]
    fn handle_sets_and_event_queues_are_context_bound() {
        let mut context =
            ExecutionContext::new(identity(10), AddressSpace::kernel_bootstrap(), 8).unwrap();
        let handle = FormHandle {
            id: 7,
            parent_id: 0,
            cfc: identity(10).cfc,
            requester: Fin::from_u128(99),
            target: identity(10).form,
            dimension: identity(10).dimension,
            operations: Operations::EXECUTE,
            valid_until_tick: 100,
            revoked: false,
        };
        context.attach_handle(handle).unwrap();
        assert_eq!(context.handles().count(), 1);
        let event = ExecutionEvent {
            source: Fin::from_u128(20),
            code: 3,
            value: 42,
        };
        context.push_event(event).unwrap();
        assert_eq!(context.pending_events(), 1);
        assert_eq!(context.pop_event(), Some(event));
        let _ = Authority::Operator;
    }

    #[test]
    fn executable_runtime_finishes_with_an_inspectable_result_and_reuses_capacity() {
        let mut scheduler = Scheduler::new();
        let mut context =
            ExecutionContext::new(identity(10), AddressSpace::kernel_bootstrap(), 8).unwrap();
        context.set_runtime(ExecutionRuntime::ExpPython);
        let first = scheduler.admit(context).unwrap();
        assert_eq!(scheduler.dispatch_next(), Ok(first));
        scheduler.finish(first, 7).unwrap();
        let completed = scheduler.context(first).unwrap();
        assert_eq!(completed.runtime(), ExecutionRuntime::ExpPython);
        assert_eq!(completed.state(), ExecutionState::Exited);
        assert_eq!(completed.cpu_state().accumulator, 7);

        for form in 20..(20 + MAX_EXECUTION_CONTEXTS as u128) {
            scheduler
                .admit(
                    ExecutionContext::new(identity(form), AddressSpace::kernel_bootstrap(), 8)
                        .unwrap(),
                )
                .unwrap();
        }
    }
}
