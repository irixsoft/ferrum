use ferrum_platform::ProcStat;
use std::time::Duration;

pub fn percent(prev: &ProcStat, next: &ProcStat) -> f64 {
    let total = next.total_ticks.saturating_sub(prev.total_ticks);
    if total == 0 {
        return 0.0;
    }
    let busy = next.busy_ticks.saturating_sub(prev.busy_ticks);
    (busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
}

/// Percent of one core, so a unit on two cores can read 200 like its `CPUQuota`.
pub fn cgroup_percent(prev_usec: u64, next_usec: u64, elapsed: Duration) -> f64 {
    let elapsed_usec = elapsed.as_micros() as f64;
    if elapsed_usec <= 0.0 {
        return 0.0;
    }
    let used = next_usec.saturating_sub(prev_usec) as f64;
    (used / elapsed_usec * 100.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_percent_is_the_busy_share_of_the_delta() {
        let prev = ProcStat {
            busy_ticks: 1000,
            total_ticks: 4000,
        };
        let next = ProcStat {
            busy_ticks: 1300,
            total_ticks: 5000,
        };
        assert_eq!(percent(&prev, &next), 30.0);
        assert_eq!(percent(&next, &next), 0.0);
        assert_eq!(percent(&next, &prev), 0.0);
    }

    #[test]
    fn cgroup_percent_is_of_one_core_and_may_exceed_a_hundred() {
        let ten = Duration::from_secs(10);
        assert_eq!(cgroup_percent(0, 2_500_000, ten), 25.0);
        assert_eq!(cgroup_percent(1_000_000, 16_000_000, ten), 150.0);
        assert_eq!(cgroup_percent(5, 5, ten), 0.0);
        assert_eq!(cgroup_percent(9, 5, ten), 0.0);
        assert_eq!(cgroup_percent(0, 5, Duration::ZERO), 0.0);
    }
}
