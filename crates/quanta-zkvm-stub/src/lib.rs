//! No-op stub of the upstream `quanta` crate.
//!
//! Why this exists: reth-primitives-traits re-exports `quanta::Instant` as `FastInstant` and
//! several reth crates we depend on (notably `reth-evm`'s metrics and `reth-trie`'s stats) use
//! it for internal duration tracking. Upstream quanta routes every non-Windows, non-wasm32
//! target through `clocks/monotonic/unix.rs`, which calls `libc::clock_gettime` against
//! `libc::timespec` — neither symbol is exposed by the SP1 zkVM libc, so the guest fails to
//! compile.
//!
//! Wall-clock timing inside the zkVM is meaningless anyway: there is no real clock and the
//! resulting `Duration` is never observed by the host. This stub provides just enough API
//! surface for the reth call sites to compile, with every measurement reported as zero.
//!
//! Only `Instant::{now, elapsed, duration_since}` are actually called from the reth crates we
//! pull in (`reth-evm/src/metrics.rs`, `reth-trie/src/stats.rs`). The other inherent methods
//! exist for forward compatibility with the upstream API surface so that any future reth call
//! site that touches `FastInstant` keeps compiling. Add more methods here only when a real
//! consumer needs them — see the upstream quanta `Instant` docs for the canonical signatures.

#![no_std]

use core::time::Duration;

/// Drop-in replacement for `quanta::Clock`. Both `metrics-util` and `metrics-exporter-prometheus`
/// (which are pulled in transitively by the workspace through reth) use this — primarily to
/// stamp idle-timeout decisions. With timing always zero, recency-tracking simply never expires
/// entries, which is the desired no-op behavior for the zkVM (and benign on the host).
#[derive(Debug, Clone, Default)]
pub struct Clock;

impl Clock {
    #[inline]
    pub fn new() -> Self {
        Self
    }

    #[inline]
    pub fn now(&self) -> Instant {
        Instant
    }
}

/// Drop-in replacement for `quanta::Instant`. Every reading is fixed at "now", so all elapsed
/// durations are `Duration::ZERO`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Default)]
pub struct Instant;

impl Instant {
    #[inline]
    pub fn now() -> Self {
        Self
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        Duration::ZERO
    }

    #[inline]
    pub fn duration_since(&self, _earlier: Self) -> Duration {
        Duration::ZERO
    }

    #[inline]
    pub fn checked_duration_since(&self, _earlier: Self) -> Option<Duration> {
        Some(Duration::ZERO)
    }

    #[inline]
    pub fn saturating_duration_since(&self, _earlier: Self) -> Duration {
        Duration::ZERO
    }

    #[inline]
    pub fn checked_add(&self, _duration: Duration) -> Option<Self> {
        Some(Self)
    }

    #[inline]
    pub fn checked_sub(&self, _duration: Duration) -> Option<Self> {
        Some(Self)
    }

    #[inline]
    pub fn as_u64(&self) -> u64 {
        0
    }
}

impl core::ops::Add<Duration> for Instant {
    type Output = Self;
    #[inline]
    fn add(self, _rhs: Duration) -> Self {
        self
    }
}

impl core::ops::Sub<Duration> for Instant {
    type Output = Self;
    #[inline]
    fn sub(self, _rhs: Duration) -> Self {
        self
    }
}

impl core::ops::Sub<Instant> for Instant {
    type Output = Duration;
    #[inline]
    fn sub(self, _rhs: Instant) -> Duration {
        Duration::ZERO
    }
}

impl core::ops::AddAssign<Duration> for Instant {
    #[inline]
    fn add_assign(&mut self, _rhs: Duration) {}
}

impl core::ops::SubAssign<Duration> for Instant {
    #[inline]
    fn sub_assign(&mut self, _rhs: Duration) {}
}
