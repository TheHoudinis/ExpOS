//! Bounded kernel control, event, and resource primitives.
//!
//! The split between tunables, readiness queues, and per-context ceilings is
//! inspired by long-standing FreeBSD design patterns. No FreeBSD source or ABI
//! is copied: this implementation is native to ExpOS, every table is
//! fixed-capacity, every mutation is validated, and the shell applies its
//! authenticated session-authority boundary before exposing mutations.

use core::cmp;

pub const MAX_TUNABLES: usize = 9;
pub const MAX_WATCHES: usize = 16;
pub const MAX_READY_EVENTS: usize = 16;
pub const RESOURCE_COUNT: usize = 5;

const NODE_EVENT_BATCH: usize = 2;
const NODE_EVENT_COALESCE: usize = 3;
const NODE_EVENT_TRACE: usize = 4;
const NODE_EVENT_DROPPED: usize = 5;
const NODE_EVENT_COALESCED: usize = 6;
const NODE_EXPBUDGET_DENIALS: usize = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunableValue {
    Bool(bool),
    Unsigned(u64),
    Text(&'static str),
}

impl TunableValue {
    pub const fn kind_name(self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Unsigned(_) => "u64",
            Self::Text(_) => "text",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunableAccess {
    ReadOnly,
    OperatorWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TunableNode {
    pub name: &'static str,
    pub value: TunableValue,
    pub access: TunableAccess,
    minimum: u64,
    maximum: u64,
}

impl TunableNode {
    const fn read_only(name: &'static str, value: TunableValue) -> Self {
        Self {
            name,
            value,
            access: TunableAccess::ReadOnly,
            minimum: 0,
            maximum: u64::MAX,
        }
    }

    const fn unsigned(name: &'static str, value: u64, minimum: u64, maximum: u64) -> Self {
        Self {
            name,
            value: TunableValue::Unsigned(value),
            access: TunableAccess::OperatorWrite,
            minimum,
            maximum,
        }
    }

    const fn boolean(name: &'static str, value: bool) -> Self {
        Self {
            name,
            value: TunableValue::Bool(value),
            access: TunableAccess::OperatorWrite,
            minimum: 0,
            maximum: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    UnknownNode,
    ReadOnly,
    InvalidType,
    OutOfRange,
    DuplicateWatch,
    WatchTableFull,
    WatchNotFound,
    ReadyQueueFull,
    InvalidLimit,
    ResourceExhausted,
    ManagedResource,
    ReleaseExceedsUsage,
}

impl ControlError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownNode => "unknown tunable",
            Self::ReadOnly => "node is read-only",
            Self::InvalidType => "value has the wrong type",
            Self::OutOfRange => "value is outside the allowed range",
            Self::DuplicateWatch => "event watch already exists",
            Self::WatchTableFull => "event watch table is full",
            Self::WatchNotFound => "event watch was not found",
            Self::ReadyQueueFull => "ready event queue is full",
            Self::InvalidLimit => "limits must satisfy used <= soft <= hard <= ceiling",
            Self::ResourceExhausted => "resource soft limit reached",
            Self::ManagedResource => "resource usage is managed by its owning subsystem",
            Self::ReleaseExceedsUsage => "release exceeds current resource usage",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TunableRegistry {
    nodes: [TunableNode; MAX_TUNABLES],
}

impl TunableRegistry {
    pub const fn new() -> Self {
        Self {
            nodes: [
                TunableNode::read_only("kern.control.abi", TunableValue::Text("Form-native-v1")),
                TunableNode::read_only("security.capability.enforced", TunableValue::Bool(true)),
                TunableNode::unsigned("kern.event.batch", 4, 1, 16),
                TunableNode::boolean("kern.event.coalesce", true),
                TunableNode::boolean("kern.event.trace", false),
                TunableNode::read_only("kern.event.dropped", TunableValue::Unsigned(0)),
                TunableNode::read_only("kern.event.coalesced", TunableValue::Unsigned(0)),
                TunableNode::read_only("kern.expbudget.denials", TunableValue::Unsigned(0)),
                TunableNode::read_only(
                    "kern.event.capacity",
                    TunableValue::Unsigned(MAX_WATCHES as u64),
                ),
            ],
        }
    }

    pub fn visit(&self, mut visitor: impl FnMut(TunableNode)) {
        for node in self.nodes {
            visitor(node);
        }
    }

    pub fn get(&self, name: &str) -> Result<TunableNode, ControlError> {
        self.nodes
            .iter()
            .find(|node| node.name == name)
            .copied()
            .ok_or(ControlError::UnknownNode)
    }

    pub fn set_text(&mut self, name: &str, raw: &str) -> Result<TunableNode, ControlError> {
        let index = self
            .nodes
            .iter()
            .position(|node| node.name == name)
            .ok_or(ControlError::UnknownNode)?;
        let node = &mut self.nodes[index];
        if node.access == TunableAccess::ReadOnly {
            return Err(ControlError::ReadOnly);
        }
        node.value = match node.value {
            TunableValue::Bool(_) => TunableValue::Bool(parse_bool(raw)?),
            TunableValue::Unsigned(_) => {
                let value = raw.parse::<u64>().map_err(|_| ControlError::InvalidType)?;
                if value < node.minimum || value > node.maximum {
                    return Err(ControlError::OutOfRange);
                }
                TunableValue::Unsigned(value)
            }
            TunableValue::Text(_) => return Err(ControlError::ReadOnly),
        };
        Ok(*node)
    }

    const fn unsigned(&self, index: usize) -> u64 {
        match self.nodes[index].value {
            TunableValue::Unsigned(value) => value,
            _ => 0,
        }
    }

    const fn boolean(&self, index: usize) -> bool {
        matches!(self.nodes[index].value, TunableValue::Bool(true))
    }

    fn set_counter(&mut self, index: usize, value: u64) {
        self.nodes[index].value = TunableValue::Unsigned(value);
    }
}

impl Default for TunableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_bool(raw: &str) -> Result<bool, ControlError> {
    match raw {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(ControlError::InvalidType),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventFilter {
    Signal,
    Timer,
    Resource,
}

impl EventFilter {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Timer => "timer",
            Self::Resource => "resource",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventWatch {
    pub identifier: u32,
    pub filter: EventFilter,
    pub deadline: u64,
    pub interval: u64,
    pub enabled: bool,
}

impl EventWatch {
    pub const fn signal(identifier: u32) -> Self {
        Self {
            identifier,
            filter: EventFilter::Signal,
            deadline: 0,
            interval: 0,
            enabled: true,
        }
    }

    pub const fn timer(identifier: u32, deadline: u64, interval: u64) -> Self {
        Self {
            identifier,
            filter: EventFilter::Timer,
            deadline,
            interval,
            enabled: true,
        }
    }

    pub const fn resource(resource: Resource) -> Self {
        Self {
            identifier: resource as u32,
            filter: EventFilter::Resource,
            deadline: 0,
            interval: 0,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyEvent {
    pub identifier: u32,
    pub filter: EventFilter,
    pub data: u64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalOutcome {
    Queued,
    Coalesced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventStats {
    pub watches: usize,
    pub pending: usize,
    pub delivered: u64,
    pub dropped: u64,
    pub coalesced: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventQueue {
    watches: [Option<EventWatch>; MAX_WATCHES],
    ready: [Option<ReadyEvent>; MAX_READY_EVENTS],
    ready_head: usize,
    ready_count: usize,
    next_sequence: u64,
    delivered: u64,
    dropped: u64,
    coalesced: u64,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self {
            watches: [None; MAX_WATCHES],
            ready: [None; MAX_READY_EVENTS],
            ready_head: 0,
            ready_count: 0,
            next_sequence: 1,
            delivered: 0,
            dropped: 0,
            coalesced: 0,
        }
    }

    pub fn register(&mut self, watch: EventWatch) -> Result<(), ControlError> {
        if self
            .watches
            .iter()
            .flatten()
            .any(|current| same_event_key(*current, watch.identifier, watch.filter))
        {
            return Err(ControlError::DuplicateWatch);
        }
        let slot = self
            .watches
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ControlError::WatchTableFull)?;
        *slot = Some(watch);
        Ok(())
    }

    pub fn unregister(&mut self, identifier: u32, filter: EventFilter) -> Result<(), ControlError> {
        let slot = self
            .watches
            .iter_mut()
            .find(|slot| slot.is_some_and(|watch| same_event_key(watch, identifier, filter)))
            .ok_or(ControlError::WatchNotFound)?;
        *slot = None;
        self.remove_ready(identifier, filter);
        Ok(())
    }

    pub fn visit_watches(&self, mut visitor: impl FnMut(EventWatch)) {
        for watch in self.watches.iter().flatten() {
            visitor(*watch);
        }
    }

    fn signal(
        &mut self,
        identifier: u32,
        filter: EventFilter,
        data: u64,
        coalesce: bool,
    ) -> Result<SignalOutcome, ControlError> {
        if !self
            .watches
            .iter()
            .flatten()
            .any(|watch| watch.enabled && same_event_key(*watch, identifier, filter))
        {
            return Err(ControlError::WatchNotFound);
        }
        if coalesce {
            for offset in 0..self.ready_count {
                let index = (self.ready_head + offset) % MAX_READY_EVENTS;
                if let Some(event) = self.ready[index].as_mut() {
                    if event.identifier == identifier && event.filter == filter {
                        event.data = if filter == EventFilter::Timer {
                            event.data.saturating_add(data)
                        } else {
                            data
                        };
                        self.coalesced = self.coalesced.saturating_add(1);
                        return Ok(SignalOutcome::Coalesced);
                    }
                }
            }
        }
        if self.ready_count == MAX_READY_EVENTS {
            self.dropped = self.dropped.saturating_add(1);
            return Err(ControlError::ReadyQueueFull);
        }
        let tail = (self.ready_head + self.ready_count) % MAX_READY_EVENTS;
        self.ready[tail] = Some(ReadyEvent {
            identifier,
            filter,
            data,
            sequence: self.next_sequence,
        });
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.ready_count += 1;
        Ok(SignalOutcome::Queued)
    }

    fn advance_timers(&mut self, now: u64, coalesce: bool) {
        for index in 0..MAX_WATCHES {
            let Some(mut watch) = self.watches[index] else {
                continue;
            };
            if !watch.enabled || watch.filter != EventFilter::Timer || now < watch.deadline {
                continue;
            }
            let elapsed = now.saturating_sub(watch.deadline);
            let (expirations, next_deadline) = match elapsed.checked_div(watch.interval) {
                None => (1, None),
                Some(expired_intervals) => match expired_intervals.checked_add(1) {
                    Some(expirations) => (
                        expirations,
                        watch
                            .interval
                            .checked_mul(expirations)
                            .and_then(|delta| watch.deadline.checked_add(delta)),
                    ),
                    None => (u64::MAX, None),
                },
            };
            match next_deadline {
                Some(deadline) => watch.deadline = deadline,
                None => watch.enabled = false,
            }
            if self
                .signal(watch.identifier, EventFilter::Timer, expirations, coalesce)
                .is_ok()
            {
                self.watches[index] = Some(watch);
            }
        }
    }

    fn poll(&mut self, output: &mut [ReadyEvent], budget: usize) -> usize {
        let amount = cmp::min(cmp::min(output.len(), budget), self.ready_count);
        for destination in output.iter_mut().take(amount) {
            *destination = self.ready[self.ready_head]
                .take()
                .expect("the ready ring count must describe occupied slots");
            self.ready_head = (self.ready_head + 1) % MAX_READY_EVENTS;
            self.ready_count -= 1;
        }
        self.delivered = self.delivered.saturating_add(amount as u64);
        amount
    }

    fn remove_ready(&mut self, identifier: u32, filter: EventFilter) {
        let mut retained = [None; MAX_READY_EVENTS];
        let mut retained_count = 0;
        for offset in 0..self.ready_count {
            let index = (self.ready_head + offset) % MAX_READY_EVENTS;
            let Some(event) = self.ready[index].take() else {
                continue;
            };
            if event.identifier != identifier || event.filter != filter {
                retained[retained_count] = Some(event);
                retained_count += 1;
            }
        }
        self.ready = retained;
        self.ready_head = 0;
        self.ready_count = retained_count;
    }

    pub fn stats(&self) -> EventStats {
        EventStats {
            watches: self.watches.iter().flatten().count(),
            pending: self.ready_count,
            delivered: self.delivered,
            dropped: self.dropped,
            coalesced: self.coalesced,
        }
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn same_event_key(watch: EventWatch, identifier: u32, filter: EventFilter) -> bool {
    watch.identifier == identifier && watch.filter == filter
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Resource {
    EventWatches = 0,
    IpcBytes = 1,
    ScratchPages = 2,
    FormOperations = 3,
    FormBytes = 4,
}

impl Resource {
    pub const ALL: [Self; RESOURCE_COUNT] = [
        Self::EventWatches,
        Self::IpcBytes,
        Self::ScratchPages,
        Self::FormOperations,
        Self::FormBytes,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::EventWatches => "event-watches",
            Self::IpcBytes => "ipc-bytes",
            Self::ScratchPages => "scratch-pages",
            Self::FormOperations => "form-operations",
            Self::FormBytes => "form-bytes",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|resource| resource.name() == raw)
            .copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimit {
    pub resource: Resource,
    pub used: u64,
    pub soft: u64,
    pub hard: u64,
    pub ceiling: u64,
    pub denials: u64,
}

impl ResourceLimit {
    const fn new(resource: Resource, soft: u64, hard: u64, ceiling: u64) -> Self {
        Self {
            resource,
            used: 0,
            soft,
            hard,
            ceiling,
            denials: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Runtime-local ExpBudget enforcement for the current shell/CFC runtime. The
/// scheduler migration will move this same checked accounting to every
/// admitted Form context rather than keeping one shell-owned instance.
pub struct ExpBudget {
    limits: [ResourceLimit; RESOURCE_COUNT],
    total_denials: u64,
}

impl ExpBudget {
    pub const fn new() -> Self {
        Self {
            limits: [
                ResourceLimit::new(Resource::EventWatches, 8, 16, MAX_WATCHES as u64),
                ResourceLimit::new(Resource::IpcBytes, 4096, 16_384, 65_536),
                ResourceLimit::new(Resource::ScratchPages, 16, 64, 256),
                ResourceLimit::new(Resource::FormOperations, 64, 256, 4096),
                ResourceLimit::new(Resource::FormBytes, 6144, 6144, 6144),
            ],
            total_denials: 0,
        }
    }

    pub fn visit(&self, mut visitor: impl FnMut(ResourceLimit)) {
        for limit in self.limits {
            visitor(limit);
        }
    }

    pub fn get(&self, resource: Resource) -> ResourceLimit {
        self.limits[resource as usize]
    }

    pub fn set(
        &mut self,
        resource: Resource,
        soft: u64,
        hard: u64,
    ) -> Result<ResourceLimit, ControlError> {
        let limit = &mut self.limits[resource as usize];
        if limit.used > soft || soft > hard || hard > limit.ceiling {
            return Err(ControlError::InvalidLimit);
        }
        limit.soft = soft;
        limit.hard = hard;
        Ok(*limit)
    }

    pub fn charge(&mut self, resource: Resource, amount: u64) -> Result<u64, ControlError> {
        let limit = &mut self.limits[resource as usize];
        let Some(next) = limit.used.checked_add(amount) else {
            limit.denials = limit.denials.saturating_add(1);
            self.total_denials = self.total_denials.saturating_add(1);
            return Err(ControlError::ResourceExhausted);
        };
        if next > limit.soft {
            limit.denials = limit.denials.saturating_add(1);
            self.total_denials = self.total_denials.saturating_add(1);
            return Err(ControlError::ResourceExhausted);
        }
        limit.used = next;
        Ok(next)
    }

    pub fn release(&mut self, resource: Resource, amount: u64) -> Result<u64, ControlError> {
        let limit = &mut self.limits[resource as usize];
        if amount > limit.used {
            return Err(ControlError::ReleaseExceedsUsage);
        }
        limit.used -= amount;
        Ok(limit.used)
    }

    pub const fn total_denials(&self) -> u64 {
        self.total_denials
    }
}

impl Default for ExpBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelControls {
    tunables: TunableRegistry,
    events: EventQueue,
    resources: ExpBudget,
}

impl KernelControls {
    pub const fn new() -> Self {
        Self {
            tunables: TunableRegistry::new(),
            events: EventQueue::new(),
            resources: ExpBudget::new(),
        }
    }

    pub const fn tunables(&self) -> &TunableRegistry {
        &self.tunables
    }

    pub const fn resources(&self) -> &ExpBudget {
        &self.resources
    }

    pub const fn events(&self) -> &EventQueue {
        &self.events
    }

    pub fn set_tunable(&mut self, name: &str, value: &str) -> Result<TunableNode, ControlError> {
        self.tunables.set_text(name, value)
    }

    pub fn register_watch(&mut self, watch: EventWatch) -> Result<(), ControlError> {
        self.charge_internal(Resource::EventWatches, 1)?;
        if let Err(error) = self.events.register(watch) {
            let _ = self.resources.release(Resource::EventWatches, 1);
            return Err(error);
        }
        Ok(())
    }

    pub fn unregister_watch(
        &mut self,
        identifier: u32,
        filter: EventFilter,
    ) -> Result<(), ControlError> {
        self.events.unregister(identifier, filter)?;
        self.resources.release(Resource::EventWatches, 1)?;
        Ok(())
    }

    pub fn signal(&mut self, identifier: u32, data: u64) -> Result<(), ControlError> {
        let coalesce = self.tunables.boolean(NODE_EVENT_COALESCE);
        let result = self
            .events
            .signal(identifier, EventFilter::Signal, data, coalesce);
        self.sync_counters();
        result.map(|_| ())
    }

    pub fn poll(&mut self, now: u64, output: &mut [ReadyEvent]) -> usize {
        let coalesce = self.tunables.boolean(NODE_EVENT_COALESCE);
        self.events.advance_timers(now, coalesce);
        let budget = self.tunables.unsigned(NODE_EVENT_BATCH) as usize;
        let amount = self.events.poll(output, budget);
        self.sync_counters();
        amount
    }

    pub fn set_limit(
        &mut self,
        resource: Resource,
        soft: u64,
        hard: u64,
    ) -> Result<ResourceLimit, ControlError> {
        self.resources.set(resource, soft, hard)
    }

    pub fn charge(&mut self, resource: Resource, amount: u64) -> Result<u64, ControlError> {
        if matches!(
            resource,
            Resource::EventWatches | Resource::FormOperations | Resource::FormBytes
        ) {
            return Err(ControlError::ManagedResource);
        }
        self.charge_internal(resource, amount)
    }

    fn charge_internal(&mut self, resource: Resource, amount: u64) -> Result<u64, ControlError> {
        match self.resources.charge(resource, amount) {
            Ok(used) => Ok(used),
            Err(error) => {
                let coalesce = self.tunables.boolean(NODE_EVENT_COALESCE);
                let _ =
                    self.events
                        .signal(resource as u32, EventFilter::Resource, amount, coalesce);
                self.sync_counters();
                Err(error)
            }
        }
    }

    pub fn release(&mut self, resource: Resource, amount: u64) -> Result<u64, ControlError> {
        if matches!(
            resource,
            Resource::EventWatches | Resource::FormOperations | Resource::FormBytes
        ) {
            return Err(ControlError::ManagedResource);
        }
        self.resources.release(resource, amount)
    }

    /// Only owning subsystems may charge actual work or resident Form content.
    pub(crate) fn charge_form_operation(&mut self) -> Result<(), ControlError> {
        self.charge_internal(Resource::FormOperations, 1)
            .map(|_| ())
    }

    /// Validate growth before publishing a content replacement; shrinking always
    /// releases the corresponding owned bytes. Manual reservations cannot forge it.
    pub(crate) fn resize_form_content(
        &mut self,
        old: usize,
        new: usize,
    ) -> Result<(), ControlError> {
        if new >= old {
            self.charge_internal(Resource::FormBytes, (new - old) as u64)
                .map(|_| ())
        } else {
            self.resources
                .release(Resource::FormBytes, (old - new) as u64)
                .map(|_| ())
        }
    }

    pub const fn tracing_enabled(&self) -> bool {
        self.tunables.boolean(NODE_EVENT_TRACE)
    }

    fn sync_counters(&mut self) {
        let stats = self.events.stats();
        self.tunables.set_counter(NODE_EVENT_DROPPED, stats.dropped);
        self.tunables
            .set_counter(NODE_EVENT_COALESCED, stats.coalesced);
        self.tunables
            .set_counter(NODE_EXPBUDGET_DENIALS, self.resources.total_denials());
    }
}

impl Default for KernelControls {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ControlError, EventFilter, EventQueue, EventWatch, KernelControls, ReadyEvent, Resource,
        TunableAccess, TunableRegistry, TunableValue, MAX_READY_EVENTS, MAX_WATCHES,
    };

    const EMPTY_EVENT: ReadyEvent = ReadyEvent {
        identifier: 0,
        filter: EventFilter::Signal,
        data: 0,
        sequence: 0,
    };

    #[test]
    fn real_form_content_and_work_are_owned_and_fail_atomically() {
        let mut controls = KernelControls::new();
        controls.set_limit(Resource::FormBytes, 4, 8).unwrap();
        controls.resize_form_content(0, 4).unwrap();
        assert_eq!(
            controls.resize_form_content(4, 5),
            Err(ControlError::ResourceExhausted)
        );
        assert_eq!(controls.resources().get(Resource::FormBytes).used, 4);
        assert_eq!(
            controls.release(Resource::FormBytes, 4),
            Err(ControlError::ManagedResource)
        );
        controls.resize_form_content(4, 1).unwrap();
        assert_eq!(controls.resources().get(Resource::FormBytes).used, 1);
        controls.set_limit(Resource::FormOperations, 1, 2).unwrap();
        controls.charge_form_operation().unwrap();
        assert_eq!(
            controls.charge_form_operation(),
            Err(ControlError::ResourceExhausted)
        );
        assert_eq!(
            controls.release(Resource::FormOperations, 1),
            Err(ControlError::ManagedResource)
        );
    }

    #[test]
    fn tunables_are_typed_validated_and_protect_read_only_nodes() {
        let mut registry = TunableRegistry::new();
        assert_eq!(
            registry.get("security.capability.enforced").unwrap().value,
            TunableValue::Bool(true)
        );
        assert_eq!(
            registry.get("security.capability.enforced").unwrap().access,
            TunableAccess::ReadOnly
        );
        assert_eq!(
            registry.set_text("security.capability.enforced", "false"),
            Err(ControlError::ReadOnly)
        );
        assert_eq!(
            registry.set_text("kern.event.batch", "0"),
            Err(ControlError::OutOfRange)
        );
        assert_eq!(
            registry.set_text("kern.event.batch", "fast"),
            Err(ControlError::InvalidType)
        );
        assert_eq!(
            registry.set_text("kern.event.batch", "8").unwrap().value,
            TunableValue::Unsigned(8)
        );
        assert_eq!(
            registry
                .set_text("kern.event.coalesce", "off")
                .unwrap()
                .value,
            TunableValue::Bool(false)
        );
    }

    #[test]
    fn signals_are_ordered_and_optionally_coalesced() {
        let mut controls = KernelControls::new();
        controls.register_watch(EventWatch::signal(7)).unwrap();
        controls.signal(7, 10).unwrap();
        controls.signal(7, 20).unwrap();
        let mut output = [EMPTY_EVENT; 4];
        assert_eq!(controls.poll(0, &mut output), 1);
        assert_eq!(output[0].identifier, 7);
        assert_eq!(output[0].data, 20);
        assert_eq!(controls.events().stats().coalesced, 1);

        controls
            .set_tunable("kern.event.coalesce", "false")
            .unwrap();
        controls.signal(7, 30).unwrap();
        controls.signal(7, 40).unwrap();
        assert_eq!(controls.poll(0, &mut output), 2);
        assert_eq!(output[0].data, 30);
        assert_eq!(output[1].data, 40);
        assert!(output[1].sequence > output[0].sequence);
    }

    #[test]
    fn dispatch_budget_is_controlled_by_tunable() {
        let mut controls = KernelControls::new();
        controls
            .set_tunable("kern.event.coalesce", "false")
            .unwrap();
        controls.set_tunable("kern.event.batch", "2").unwrap();
        controls.register_watch(EventWatch::signal(3)).unwrap();
        for value in 0..4 {
            controls.signal(3, value).unwrap();
        }
        let mut output = [EMPTY_EVENT; MAX_READY_EVENTS];
        assert_eq!(controls.poll(0, &mut output), 2);
        assert_eq!(controls.events().stats().pending, 2);
    }

    #[test]
    fn timer_watches_report_missed_expirations_and_rearm() {
        let mut controls = KernelControls::new();
        controls
            .register_watch(EventWatch::timer(9, 100, 25))
            .unwrap();
        let mut output = [EMPTY_EVENT; 2];
        assert_eq!(controls.poll(99, &mut output), 0);
        assert_eq!(controls.poll(151, &mut output), 1);
        assert_eq!(output[0].filter, EventFilter::Timer);
        assert_eq!(output[0].data, 3);
        assert_eq!(controls.poll(174, &mut output), 0);
        assert_eq!(controls.poll(175, &mut output), 1);
    }

    #[test]
    fn pending_timer_readiness_accumulates_expirations() {
        let mut controls = KernelControls::new();
        controls
            .register_watch(EventWatch::timer(12, 100, 25))
            .unwrap();
        let mut no_output = [];
        assert_eq!(controls.poll(100, &mut no_output), 0);
        let mut output = [EMPTY_EVENT; 1];
        assert_eq!(controls.poll(150, &mut output), 1);
        assert_eq!(output[0].data, 3);
        assert_eq!(controls.events().stats().coalesced, 1);
    }

    #[test]
    fn watch_limit_is_enforced_and_released_on_delete() {
        let mut controls = KernelControls::new();
        for identifier in 0..8 {
            controls
                .register_watch(EventWatch::signal(identifier))
                .unwrap();
        }
        assert_eq!(
            controls.register_watch(EventWatch::signal(99)),
            Err(ControlError::ResourceExhausted)
        );
        controls.unregister_watch(0, EventFilter::Signal).unwrap();
        controls.register_watch(EventWatch::signal(99)).unwrap();
        assert_eq!(controls.resources().get(Resource::EventWatches).used, 8);
        assert_eq!(MAX_WATCHES, 16);
    }

    #[test]
    fn resource_limits_validate_account_and_emit_denial_readiness() {
        let mut controls = KernelControls::new();
        controls
            .register_watch(EventWatch::resource(Resource::ScratchPages))
            .unwrap();
        controls.set_limit(Resource::ScratchPages, 2, 4).unwrap();
        assert_eq!(controls.charge(Resource::ScratchPages, 2), Ok(2));
        assert_eq!(
            controls.charge(Resource::ScratchPages, 1),
            Err(ControlError::ResourceExhausted)
        );
        let mut output = [EMPTY_EVENT; 2];
        assert_eq!(controls.poll(0, &mut output), 1);
        assert_eq!(output[0].filter, EventFilter::Resource);
        assert_eq!(output[0].identifier, Resource::ScratchPages as u32);
        assert_eq!(output[0].data, 1);
        assert_eq!(controls.release(Resource::ScratchPages, 1), Ok(1));
        assert_eq!(
            controls.release(Resource::ScratchPages, 2),
            Err(ControlError::ReleaseExceedsUsage)
        );
        assert_eq!(
            controls.set_limit(Resource::ScratchPages, 0, 4),
            Err(ControlError::InvalidLimit)
        );
        assert_eq!(
            controls
                .tunables()
                .get("kern.expbudget.denials")
                .unwrap()
                .value,
            TunableValue::Unsigned(1)
        );
    }

    #[test]
    fn duplicate_and_missing_watches_are_rejected() {
        let mut controls = KernelControls::new();
        controls.register_watch(EventWatch::signal(4)).unwrap();
        assert_eq!(
            controls.register_watch(EventWatch::signal(4)),
            Err(ControlError::DuplicateWatch)
        );
        assert_eq!(
            controls.unregister_watch(5, EventFilter::Signal),
            Err(ControlError::WatchNotFound)
        );
    }

    #[test]
    fn event_watch_usage_cannot_be_forged_through_the_public_ledger() {
        let mut controls = KernelControls::new();
        controls.register_watch(EventWatch::signal(1)).unwrap();
        assert_eq!(
            controls.charge(Resource::EventWatches, 1),
            Err(ControlError::ManagedResource)
        );
        assert_eq!(
            controls.release(Resource::EventWatches, 1),
            Err(ControlError::ManagedResource)
        );
        assert_eq!(controls.resources().get(Resource::EventWatches).used, 1);
    }

    #[test]
    fn overflowing_periodic_timer_expires_once_instead_of_wrapping() {
        let mut controls = KernelControls::new();
        controls
            .register_watch(EventWatch::timer(44, u64::MAX - 1, 4))
            .unwrap();
        let mut output = [EMPTY_EVENT; 1];
        assert_eq!(controls.poll(u64::MAX, &mut output), 1);
        assert_eq!(output[0].identifier, 44);
        assert_eq!(controls.poll(u64::MAX, &mut output), 0);
    }

    #[test]
    fn event_tables_fail_closed_at_their_fixed_capacities() {
        let mut queue = EventQueue::new();
        for identifier in 0..MAX_WATCHES as u32 {
            queue.register(EventWatch::signal(identifier)).unwrap();
        }
        assert_eq!(
            queue.register(EventWatch::signal(MAX_WATCHES as u32)),
            Err(ControlError::WatchTableFull)
        );
        for value in 0..MAX_READY_EVENTS as u64 {
            queue.signal(0, EventFilter::Signal, value, false).unwrap();
        }
        assert_eq!(
            queue.signal(0, EventFilter::Signal, 99, false),
            Err(ControlError::ReadyQueueFull)
        );
        assert_eq!(queue.stats().pending, MAX_READY_EVENTS);
        assert_eq!(queue.stats().dropped, 1);
    }
}
