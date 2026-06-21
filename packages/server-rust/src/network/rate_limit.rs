//! Per-connection inbound op-rate limiting for the WebSocket data plane.
//!
//! The `tower_governor` rate limiter only guards the admin/login HTTP routes;
//! the data plane (`/ws`, `/sync`) relies on aggregate load shedding
//! (`MAX_IN_FLIGHT` + worker-inbox `Overloaded`). Load shedding bounds total
//! in-flight work but does **not** bound a single abusive peer: one connection
//! can continuously occupy the in-flight slots and saturate worker inboxes,
//! degrading every other client.
//!
//! [`TokenBucket`] adds a per-connection cap on inbound op rate so one peer's
//! flood is throttled (the client is told to back off with a 429) without
//! starving others and without tearing the connection down.
//!
//! Deliberately **not** thread-safe: each connection's Phase-2 read loop is the
//! sole owner of its bucket, so no locking is required.

use std::time::Instant;

/// A monotonic token bucket.
///
/// Tokens refill continuously at `refill_per_sec` up to `capacity` (the burst
/// ceiling). Each accepted op consumes tokens equal to its cost; when the
/// bucket cannot cover the cost the op is rejected (the caller backs the client
/// off) but the connection survives and recovers as tokens refill — "отбой, не
/// падение".
///
/// A `refill_per_sec` of 0 disables limiting entirely (every op is allowed),
/// preserving the legacy unlimited behavior when an operator opts out.
#[derive(Debug)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a full bucket as of `now`.
    ///
    /// `refill_per_sec` is the sustained ops/second; `burst` is the bucket
    /// capacity (the most ops that can be accepted instantaneously after an idle
    /// period). `burst` is floored to 1 when non-zero refill is configured so a
    /// misconfigured `burst = 0` cannot wedge the connection.
    #[must_use]
    pub fn new(refill_per_sec: u32, burst: u32, now: Instant) -> Self {
        let capacity = if refill_per_sec > 0 {
            f64::from(burst.max(1))
        } else {
            f64::from(burst)
        };
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec: f64::from(refill_per_sec),
            last_refill: now,
        }
    }

    /// Whether limiting is disabled (refill rate 0).
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.refill_per_sec <= 0.0
    }

    /// Attempts to consume `cost` tokens as of `now`. Returns `true` if the op
    /// is allowed.
    ///
    /// `cost` is clamped to `capacity` so a single legitimate op (or batch)
    /// larger than the burst ceiling is never permanently rejected — it simply
    /// drains the bucket and the next ops wait for refill.
    pub fn try_consume_at(&mut self, cost: u32, now: Instant) -> bool {
        if self.is_disabled() {
            return true;
        }
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);

        let cost = f64::from(cost).min(self.capacity);
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }

    /// Convenience wrapper over [`try_consume_at`](Self::try_consume_at) using
    /// the current instant.
    pub fn try_consume(&mut self, cost: u32) -> bool {
        self.try_consume_at(cost, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn disabled_bucket_always_allows() {
        let mut b = TokenBucket::new(0, 0, Instant::now());
        assert!(b.is_disabled());
        for _ in 0..10_000 {
            assert!(b.try_consume(1_000));
        }
    }

    #[test]
    fn allows_up_to_burst_then_throttles() {
        // Flood within a single instant: burst 10 allows 10 single-op consumes,
        // the 11th is throttled (rejected, not a panic) — отбой, не падение.
        let t0 = Instant::now();
        let mut b = TokenBucket::new(100, 10, t0);
        for i in 0..10 {
            assert!(b.try_consume_at(1, t0), "op {i} within burst must pass");
        }
        assert!(
            !b.try_consume_at(1, t0),
            "op past burst must be throttled, not crash"
        );
    }

    #[test]
    fn refills_over_time_and_recovers() {
        // After exhausting the burst, the bucket recovers as time passes — the
        // peer is slowed, never permanently shut out.
        let t0 = Instant::now();
        let mut b = TokenBucket::new(100, 10, t0); // 100 ops/sec
        for _ in 0..10 {
            assert!(b.try_consume_at(1, t0));
        }
        assert!(!b.try_consume_at(1, t0));

        // 100ms later → 10 tokens refilled (100/sec * 0.1s).
        let t1 = t0 + Duration::from_millis(100);
        for _ in 0..10 {
            assert!(b.try_consume_at(1, t1), "bucket must recover after refill");
        }
        assert!(!b.try_consume_at(1, t1));
    }

    #[test]
    fn batch_cost_drains_proportionally() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1_000, 100, t0);
        // One 60-op batch then a 40-op batch exactly drains the burst of 100.
        assert!(b.try_consume_at(60, t0));
        assert!(b.try_consume_at(40, t0));
        // Bucket empty: a further op is throttled.
        assert!(!b.try_consume_at(1, t0));
    }

    #[test]
    fn oversized_op_clamped_not_permanently_rejected() {
        // A single op whose cost exceeds the burst ceiling still passes against a
        // full bucket (cost clamped to capacity) instead of wedging forever.
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1_000, 100, t0);
        assert!(
            b.try_consume_at(10_000, t0),
            "oversized op must pass against a full bucket"
        );
        // It drained the bucket, so the next op waits for refill.
        assert!(!b.try_consume_at(1, t0));
    }

    #[test]
    fn zero_burst_with_refill_is_floored_to_one() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(100, 0, t0);
        // Floored to capacity 1 so a misconfig cannot wedge the connection.
        assert!(b.try_consume_at(1, t0));
        assert!(!b.try_consume_at(1, t0));
    }
}
