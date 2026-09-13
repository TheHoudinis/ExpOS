//! Allocation-free scanout pacing for the graphical compositor.
//!
//! This module deliberately does not program a display controller or claim
//! that a requested refresh rate is an active hardware mode. It turns a
//! caller-provided monotonic clock into exact rational frame deadlines. With
//! [`VSyncPolicy::On`], those deadlines pace framebuffer commits before the
//! Bochs/QEMU scanout backend performs its bounded vertical-retrace wait and
//! page flip. A future interrupt-driven driver can replace that polling
//! boundary without changing the cadence model. [`VSyncPolicy::Off`] keeps the
//! selected cadence but skips the retrace latch, so it cannot promise
//! tear-free presentation.

/// Refresh rates supported by the compositor's pacing policy.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RefreshRate {
    #[default]
    Hz60 = 60,
    Hz75 = 75,
    Hz120 = 120,
    Hz144 = 144,
}

impl RefreshRate {
    pub const ALL: [Self; 4] = [Self::Hz60, Self::Hz75, Self::Hz120, Self::Hz144];

    pub const fn hz(self) -> u16 {
        self as u16
    }

    pub const fn from_hz(hz: u16) -> Option<Self> {
        match hz {
            60 => Some(Self::Hz60),
            75 => Some(Self::Hz75),
            120 => Some(Self::Hz120),
            144 => Some(Self::Hz144),
            _ => None,
        }
    }

    /// Decode the stable settings representation (`0` through `3`).
    pub const fn from_persisted(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Hz60),
            1 => Some(Self::Hz75),
            2 => Some(Self::Hz120),
            3 => Some(Self::Hz144),
            _ => None,
        }
    }

    /// Encode this rate for persistent settings.
    pub const fn persisted(self) -> u8 {
        match self {
            Self::Hz60 => 0,
            Self::Hz75 => 1,
            Self::Hz120 => 2,
            Self::Hz144 => 3,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hz60 => "60 Hz",
            Self::Hz75 => "75 Hz",
            Self::Hz120 => "120 Hz",
            Self::Hz144 => "144 Hz",
        }
    }
}

/// Whether frame releases are aligned to the configured refresh cadence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VSyncPolicy {
    Off,
    #[default]
    On,
}

impl VSyncPolicy {
    pub const fn from_enabled(enabled: bool) -> Self {
        if enabled {
            Self::On
        } else {
            Self::Off
        }
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::On)
    }
}

/// A reduced, exact frame period measured in caller clock ticks.
///
/// For example, a nanosecond clock at 60 Hz produces `50_000_000 / 3`
/// ticks per frame instead of truncating every frame to `16_666_666` ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramePeriod {
    numerator_ticks: u64,
    denominator: u16,
}

impl FramePeriod {
    pub const fn numerator_ticks(self) -> u64 {
        self.numerator_ticks
    }

    pub const fn denominator(self) -> u16 {
        self.denominator
    }

    pub const fn whole_ticks(self) -> u64 {
        self.numerator_ticks / self.denominator as u64
    }

    pub const fn fractional_numerator(self) -> u16 {
        (self.numerator_ticks % self.denominator as u64) as u16
    }

    /// Smallest integral clock interval that is not shorter than the period.
    pub const fn ceiling_ticks(self) -> u64 {
        self.whole_ticks()
            .saturating_add((self.fractional_numerator() != 0) as u64)
    }
}

/// An exact absolute frame deadline on the caller's monotonic clock.
///
/// `whole_tick + fractional_numerator / fractional_denominator` is the exact
/// rational value. A timer accepting only integer ticks should wait until
/// [`not_before_tick`](Self::not_before_tick), which rounds upward and never
/// releases a synchronized frame early.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameDeadline {
    frame_index: u64,
    whole_tick: u64,
    fractional_numerator: u16,
    fractional_denominator: u16,
}

impl FrameDeadline {
    pub const fn frame_index(self) -> u64 {
        self.frame_index
    }

    pub const fn whole_tick(self) -> u64 {
        self.whole_tick
    }

    pub const fn fractional_numerator(self) -> u16 {
        self.fractional_numerator
    }

    pub const fn fractional_denominator(self) -> u16 {
        self.fractional_denominator
    }

    /// First integral clock tick at or after this rational deadline.
    pub const fn not_before_tick(self) -> u64 {
        self.whole_tick
            .saturating_add((self.fractional_numerator != 0) as u64)
    }

    pub const fn is_due(self, now_tick: u64) -> bool {
        now_tick >= self.not_before_tick()
    }
}

/// Validated timing configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimingConfig {
    refresh_rate: RefreshRate,
    vsync: VSyncPolicy,
    clock_ticks_per_second: u64,
}

impl TimingConfig {
    /// Build a configuration for a monotonic clock.
    ///
    /// The clock must distinguish at least one tick per requested frame. This
    /// keeps separate refresh slots observable and prevents ambiguous pacing.
    pub const fn new(
        refresh_rate: RefreshRate,
        vsync: VSyncPolicy,
        clock_ticks_per_second: u64,
    ) -> Result<Self, TimingError> {
        if clock_ticks_per_second == 0 {
            return Err(TimingError::ZeroClockFrequency);
        }
        if clock_ticks_per_second < refresh_rate.hz() as u64 {
            return Err(TimingError::ClockTooCoarse);
        }
        Ok(Self {
            refresh_rate,
            vsync,
            clock_ticks_per_second,
        })
    }

    pub const fn refresh_rate(self) -> RefreshRate {
        self.refresh_rate
    }

    pub const fn vsync(self) -> VSyncPolicy {
        self.vsync
    }

    pub const fn clock_ticks_per_second(self) -> u64 {
        self.clock_ticks_per_second
    }

    pub const fn period(self) -> FramePeriod {
        let divisor = gcd(self.clock_ticks_per_second, self.refresh_rate.hz() as u64);
        FramePeriod {
            numerator_ticks: self.clock_ticks_per_second / divisor,
            denominator: (self.refresh_rate.hz() as u64 / divisor) as u16,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingError {
    ZeroClockFrequency,
    ClockTooCoarse,
}

/// Why the compositor has permission to present immediately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameRelease {
    /// The configured cadence was reached, but the scanout driver will not
    /// latch the following flip to vertical retrace.
    Unsynchronized(FrameDeadline),
    /// The configured cadence reached this exact deadline.
    Synchronized(FrameDeadline),
}

/// Result of one compositor pacing decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameDecision {
    /// There is no pending damage. Due slots are treated as idle, not missed.
    Idle {
        next_deadline: Option<FrameDeadline>,
    },
    /// Keep polling or sleep until `deadline.not_before_tick()`.
    WaitUntil { deadline: FrameDeadline },
    /// Render and present now. This decision consumes one pacing slot.
    PresentNow {
        release: FrameRelease,
        missed_frames: u64,
    },
    /// No later deadline fits in the monotonic clock's `u64` range.
    ClockExhausted,
}

impl FrameDecision {
    pub const fn should_present(self) -> bool {
        matches!(self, Self::PresentNow { .. })
    }

    pub const fn wake_tick(self) -> Option<u64> {
        match self {
            Self::WaitUntil { deadline } => Some(deadline.not_before_tick()),
            _ => None,
        }
    }

    pub const fn missed_frames(self) -> u64 {
        match self {
            Self::PresentNow { missed_frames, .. } => missed_frames,
            _ => 0,
        }
    }
}

/// Saturating lifetime counters for one pacer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PacingStats {
    /// Number of `PresentNow` decisions issued to the compositor.
    pub frame_releases: u64,
    /// Refresh slots skipped while a frame was pending at either VSync policy.
    pub missed_frames: u64,
    /// Refresh slots skipped because the compositor reported no pending work.
    pub idle_frames: u64,
    /// Backwards clock observations that forced a safe phase reset.
    pub clock_discontinuities: u64,
}

/// Stateful, allocation-free compositor frame pacer.
///
/// Call [`decide`](Self::decide) only when the compositor is ready to honor a
/// returned `PresentNow`; that result consumes its cadence slot. Pass
/// `frame_pending = false` when nothing needs repainting so idle refreshes are
/// not incorrectly reported as missed frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramePacer {
    config: TimingConfig,
    epoch_tick: u64,
    next_frame_index: Option<u64>,
    last_observed_tick: u64,
    stats: PacingStats,
}

impl FramePacer {
    pub const fn new(config: TimingConfig, start_tick: u64) -> Self {
        Self {
            config,
            epoch_tick: start_tick,
            next_frame_index: Some(0),
            last_observed_tick: start_tick,
            stats: PacingStats {
                frame_releases: 0,
                missed_frames: 0,
                idle_frames: 0,
                clock_discontinuities: 0,
            },
        }
    }

    pub const fn config(&self) -> TimingConfig {
        self.config
    }

    pub const fn period(&self) -> FramePeriod {
        self.config.period()
    }

    pub const fn stats(&self) -> PacingStats {
        self.stats
    }

    /// Return an exact deadline relative to the current pacing epoch.
    ///
    /// `None` means that the absolute deadline would exceed `u64::MAX` clock
    /// ticks; it is never silently wrapped to an earlier instant.
    pub fn deadline_for(&self, frame_index: u64) -> Option<FrameDeadline> {
        deadline_from(self.epoch_tick, self.config.period(), frame_index)
    }

    /// Change refresh/VSync/timebase policy and begin a fresh phase at `now`.
    /// Lifetime statistics are intentionally retained.
    pub fn reconfigure(&mut self, config: TimingConfig, now_tick: u64) {
        self.config = config;
        self.reset_phase(now_tick);
    }

    /// Begin a fresh cadence without clearing lifetime statistics.
    pub fn reset_phase(&mut self, now_tick: u64) {
        self.epoch_tick = now_tick;
        self.next_frame_index = Some(0);
        self.last_observed_tick = now_tick;
    }

    pub fn reset_stats(&mut self) {
        self.stats = PacingStats::default();
    }

    /// Decide whether a pending compositor frame should wait or present.
    ///
    /// The timestamp must come from the monotonic clock described by
    /// [`TimingConfig::clock_ticks_per_second`]. A backwards timestamp is
    /// treated as a clock discontinuity: the phase is reset at `now_tick`
    /// rather than turning the regression into a huge missed-frame count.
    pub fn decide(&mut self, now_tick: u64, frame_pending: bool) -> FrameDecision {
        self.observe_clock(now_tick);

        let Some(next_index) = self.next_frame_index else {
            return FrameDecision::ClockExhausted;
        };
        let Some(next_deadline) = self.deadline_for(next_index) else {
            self.next_frame_index = None;
            return FrameDecision::ClockExhausted;
        };

        if !frame_pending {
            if !next_deadline.is_due(now_tick) {
                return FrameDecision::Idle {
                    next_deadline: Some(next_deadline),
                };
            }

            let latest_due = self.latest_due_index(now_tick);
            let idle = latest_due.saturating_sub(next_index).saturating_add(1);
            self.stats.idle_frames = self.stats.idle_frames.saturating_add(idle);
            self.next_frame_index = latest_due.checked_add(1);
            return FrameDecision::Idle {
                next_deadline: self
                    .next_frame_index
                    .and_then(|index| self.deadline_for(index)),
            };
        }

        if !next_deadline.is_due(now_tick) {
            return FrameDecision::WaitUntil {
                deadline: next_deadline,
            };
        }

        let latest_due = self.latest_due_index(now_tick);
        let missed_frames = latest_due.saturating_sub(next_index);
        let deadline = self
            .deadline_for(latest_due)
            .expect("a deadline no later than now must fit the clock range");
        self.stats.frame_releases = self.stats.frame_releases.saturating_add(1);
        self.stats.missed_frames = self.stats.missed_frames.saturating_add(missed_frames);
        self.next_frame_index = latest_due.checked_add(1);
        FrameDecision::PresentNow {
            release: if self.config.vsync == VSyncPolicy::On {
                FrameRelease::Synchronized(deadline)
            } else {
                FrameRelease::Unsynchronized(deadline)
            },
            missed_frames,
        }
    }

    fn latest_due_index(&self, now_tick: u64) -> u64 {
        let elapsed = now_tick.saturating_sub(self.epoch_tick);
        let scaled = elapsed as u128 * self.config.refresh_rate.hz() as u128;
        (scaled / self.config.clock_ticks_per_second as u128) as u64
    }

    fn observe_clock(&mut self, now_tick: u64) {
        if now_tick < self.last_observed_tick {
            self.stats.clock_discontinuities = self.stats.clock_discontinuities.saturating_add(1);
            self.reset_phase(now_tick);
        } else {
            self.last_observed_tick = now_tick;
        }
    }
}

fn deadline_from(epoch_tick: u64, period: FramePeriod, frame_index: u64) -> Option<FrameDeadline> {
    let product = frame_index as u128 * period.numerator_ticks as u128;
    let denominator = period.denominator as u128;
    let offset = product / denominator;
    let fractional_numerator = (product % denominator) as u16;
    if offset > (u64::MAX - epoch_tick) as u128 {
        return None;
    }

    let whole_tick = epoch_tick + offset as u64;
    if whole_tick == u64::MAX && fractional_numerator != 0 {
        return None;
    }

    Some(FrameDeadline {
        frame_index,
        whole_tick,
        fractional_numerator,
        fractional_denominator: period.denominator,
    })
}

const fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    fn config(rate: RefreshRate, vsync: VSyncPolicy) -> TimingConfig {
        TimingConfig::new(rate, vsync, NANOS_PER_SECOND).unwrap()
    }

    #[test]
    fn supported_rates_are_explicit_and_round_trip() {
        assert_eq!(RefreshRate::ALL.len(), 4);
        for (persisted, rate) in RefreshRate::ALL.into_iter().enumerate() {
            assert_eq!(RefreshRate::from_hz(rate.hz()), Some(rate));
            assert_eq!(rate.persisted(), persisted as u8);
            assert_eq!(RefreshRate::from_persisted(persisted as u8), Some(rate));
        }
        assert_eq!(RefreshRate::from_hz(59), None);
        assert_eq!(RefreshRate::from_hz(165), None);
        assert_eq!(RefreshRate::from_persisted(4), None);
        assert_eq!(RefreshRate::from_persisted(u8::MAX), None);
        assert_eq!(RefreshRate::default(), RefreshRate::Hz60);
    }

    #[test]
    fn nanosecond_periods_stay_exact_and_reduced() {
        let expected = [
            (RefreshRate::Hz60, 50_000_000, 3),
            (RefreshRate::Hz75, 40_000_000, 3),
            (RefreshRate::Hz120, 25_000_000, 3),
            (RefreshRate::Hz144, 62_500_000, 9),
        ];
        for (rate, numerator, denominator) in expected {
            let period = config(rate, VSyncPolicy::On).period();
            assert_eq!(period.numerator_ticks(), numerator);
            assert_eq!(period.denominator(), denominator);
        }
    }

    #[test]
    fn rational_deadlines_do_not_accumulate_rounding_drift() {
        for rate in RefreshRate::ALL {
            let pacer = FramePacer::new(config(rate, VSyncPolicy::On), 37);
            let one_second = pacer.deadline_for(rate.hz() as u64).unwrap();
            assert_eq!(one_second.whole_tick(), NANOS_PER_SECOND + 37);
            assert_eq!(one_second.fractional_numerator(), 0);
            assert_eq!(one_second.not_before_tick(), NANOS_PER_SECOND + 37);
        }

        let pacer = FramePacer::new(config(RefreshRate::Hz60, VSyncPolicy::On), 0);
        let first = pacer.deadline_for(1).unwrap();
        assert_eq!(first.whole_tick(), 16_666_666);
        assert_eq!(first.fractional_numerator(), 2);
        assert_eq!(first.fractional_denominator(), 3);
        assert_eq!(first.not_before_tick(), 16_666_667);
    }

    #[test]
    fn vsync_waits_until_the_ceiling_of_a_fractional_deadline() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz60, VSyncPolicy::On), 0);
        assert_eq!(
            pacer.decide(0, true),
            FrameDecision::PresentNow {
                release: FrameRelease::Synchronized(pacer.deadline_for(0).unwrap()),
                missed_frames: 0,
            }
        );
        let decision = pacer.decide(16_666_666, true);
        assert!(!decision.should_present());
        assert_eq!(decision.wake_tick(), Some(16_666_667));
        assert!(matches!(
            pacer.decide(16_666_667, true),
            FrameDecision::PresentNow {
                release: FrameRelease::Synchronized(_),
                missed_frames: 0
            }
        ));
    }

    #[test]
    fn late_frames_skip_to_the_newest_due_slot_and_count_misses() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz60, VSyncPolicy::On), 0);
        assert!(pacer.decide(0, true).should_present());
        let decision = pacer.decide(50_000_000, true);
        assert_eq!(decision.missed_frames(), 2);
        assert_eq!(
            decision,
            FrameDecision::PresentNow {
                release: FrameRelease::Synchronized(pacer.deadline_for(3).unwrap()),
                missed_frames: 2,
            }
        );
        assert_eq!(pacer.stats().frame_releases, 2);
        assert_eq!(pacer.stats().missed_frames, 2);
    }

    #[test]
    fn idle_refreshes_are_not_reported_as_missed_frames() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz60, VSyncPolicy::On), 0);
        let decision = pacer.decide(50_000_000, false);
        let FrameDecision::Idle {
            next_deadline: Some(next),
        } = decision
        else {
            panic!("expected an idle decision with a future deadline");
        };
        assert_eq!(next.frame_index(), 4);
        assert_eq!(next.not_before_tick(), 66_666_667);
        assert_eq!(pacer.stats().idle_frames, 4);
        assert_eq!(pacer.stats().missed_frames, 0);
        assert!(matches!(
            pacer.decide(50_000_000, true),
            FrameDecision::WaitUntil { .. }
        ));
    }

    #[test]
    fn vsync_off_keeps_the_rate_limit_without_claiming_retrace_sync() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz144, VSyncPolicy::Off), 10);
        let first = pacer.deadline_for(0).unwrap();
        assert_eq!(
            pacer.decide(10, true),
            FrameDecision::PresentNow {
                release: FrameRelease::Unsynchronized(first),
                missed_frames: 0,
            }
        );
        let second = pacer.deadline_for(1).unwrap();
        assert_eq!(
            pacer.decide(10, true),
            FrameDecision::WaitUntil { deadline: second }
        );
        assert_eq!(
            pacer.decide(second.not_before_tick(), true),
            FrameDecision::PresentNow {
                release: FrameRelease::Unsynchronized(second),
                missed_frames: 0,
            }
        );
        let FrameDecision::Idle {
            next_deadline: Some(next),
        } = pacer.decide(second.not_before_tick(), false)
        else {
            panic!("expected a future pacing deadline with VSync off");
        };
        assert_eq!(next.frame_index(), 2);
        assert_eq!(pacer.stats().frame_releases, 2);
        assert_eq!(pacer.stats().missed_frames, 0);
    }

    #[test]
    fn backwards_clock_reading_resets_phase_without_false_misses() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz120, VSyncPolicy::On), 1_000);
        assert!(pacer.decide(1_000, true).should_present());
        assert!(matches!(
            pacer.decide(900, true),
            FrameDecision::PresentNow {
                missed_frames: 0,
                ..
            }
        ));
        assert_eq!(pacer.stats().clock_discontinuities, 1);
        assert_eq!(pacer.stats().missed_frames, 0);
    }

    #[test]
    fn reconfiguration_rephases_at_the_supplied_timestamp() {
        let mut pacer = FramePacer::new(config(RefreshRate::Hz60, VSyncPolicy::On), 0);
        assert!(pacer.decide(0, true).should_present());
        pacer.reconfigure(config(RefreshRate::Hz144, VSyncPolicy::On), 5_000);
        assert_eq!(pacer.config().refresh_rate(), RefreshRate::Hz144);
        assert!(pacer.decide(5_000, true).should_present());
        assert_eq!(pacer.stats().frame_releases, 2);
    }

    #[test]
    fn invalid_or_ambiguous_clocks_are_rejected() {
        assert_eq!(
            TimingConfig::new(RefreshRate::Hz60, VSyncPolicy::On, 0),
            Err(TimingError::ZeroClockFrequency)
        );
        assert_eq!(
            TimingConfig::new(RefreshRate::Hz144, VSyncPolicy::On, 143),
            Err(TimingError::ClockTooCoarse)
        );
        assert!(TimingConfig::new(RefreshRate::Hz144, VSyncPolicy::On, 144).is_ok());
    }

    #[test]
    fn deadlines_never_wrap_at_the_end_of_the_clock_range() {
        let config = TimingConfig::new(RefreshRate::Hz60, VSyncPolicy::On, 60).unwrap();
        let mut pacer = FramePacer::new(config, u64::MAX);
        assert!(pacer.decide(u64::MAX, true).should_present());
        assert_eq!(pacer.decide(u64::MAX, true), FrameDecision::ClockExhausted);
        assert_eq!(pacer.deadline_for(1), None);
    }
}
