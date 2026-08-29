//! Fixed-capacity ring buffer of [`Telemetry`] samples.
//!
//! Powers the TUI sparkline and GUI chart: frontends push one sample per
//! poll tick and read back the last N values oldest→newest.

use std::collections::VecDeque;

use crate::domain::Telemetry;

/// Ring buffer holding the most recent [`Telemetry`] samples.
///
/// Pushing beyond capacity evicts the oldest sample.
pub struct History {
    capacity: usize,
    samples: VecDeque<Telemetry>,
}

impl History {
    /// Create a buffer holding at most `capacity` samples.
    ///
    /// A `capacity` of 0 is treated as 1: the buffer always retains at
    /// least the latest sample.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    /// Append a sample, evicting the oldest one when full.
    pub fn push(&mut self, t: Telemetry) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(t);
    }

    /// Number of samples currently stored.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the buffer holds no samples yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Temperature series for sensor `idx` (0..3), oldest→newest.
    ///
    /// Panic-free: an `idx` outside 0..3 returns an empty `Vec`.
    pub fn temps_series(&self, idx: usize) -> Vec<f32> {
        if idx >= 3 {
            return Vec::new();
        }
        self.samples.iter().map(|t| t.temps[idx]).collect()
    }

    /// Fan RPM series oldest→newest as `(cpu, gpu)`, widened to `u64`
    /// because ratatui's `Sparkline` consumes `&[u64]`.
    pub fn rpm_series(&self) -> (Vec<u64>, Vec<u64>) {
        self.samples
            .iter()
            .map(|t| (u64::from(t.fan_cpu_rpm), u64::from(t.fan_gpu_rpm)))
            .unzip()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct, recognizable sample: rpm and temps all derive from `n`.
    fn sample(n: u32) -> Telemetry {
        Telemetry {
            fan_cpu_rpm: 1000 + n,
            fan_gpu_rpm: 2000 + n,
            temps: [n as f32, 10.0 + n as f32, 20.0 + n as f32],
            battery_pct: 80,
            battery_status: "Discharging".to_string(),
        }
    }

    #[test]
    fn new_history_is_empty_until_first_push() {
        let mut h = History::new(4);
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
        h.push(sample(0));
        assert_eq!(h.len(), 1);
        assert!(!h.is_empty());
    }

    #[test]
    fn capacity_zero_behaves_as_one() {
        let mut h = History::new(0);
        h.push(sample(1));
        h.push(sample(2));
        assert_eq!(h.len(), 1);
        // Only the latest sample survives.
        assert_eq!(h.rpm_series(), (vec![1002], vec![2002]));
    }

    #[test]
    fn push_beyond_capacity_keeps_len_at_capacity() {
        let mut h = History::new(3);
        for n in 0..5 {
            h.push(sample(n));
        }
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn rpm_series_pairs_cpu_gpu_oldest_to_newest_after_eviction() {
        let mut h = History::new(3);
        for n in 0..5 {
            h.push(sample(n));
        }
        // Samples 0 and 1 were evicted; 2, 3, 4 remain in push order.
        let (cpu, gpu) = h.rpm_series();
        assert_eq!(cpu, vec![1002, 1003, 1004]);
        assert_eq!(gpu, vec![2002, 2003, 2004]);
    }

    #[test]
    fn temps_series_extracts_requested_sensor_in_order() {
        let mut h = History::new(4);
        for n in 0..3 {
            h.push(sample(n));
        }
        assert_eq!(h.temps_series(0), vec![0.0, 1.0, 2.0]);
        assert_eq!(h.temps_series(1), vec![10.0, 11.0, 12.0]);
        assert_eq!(h.temps_series(2), vec![20.0, 21.0, 22.0]);
    }

    #[test]
    fn temps_series_out_of_range_index_yields_empty() {
        let mut h = History::new(4);
        h.push(sample(7));
        assert!(h.temps_series(3).is_empty());
        assert!(h.temps_series(usize::MAX).is_empty());
    }
}
