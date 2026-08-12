use std::time::{Duration, Instant};

pub const WARMUP_ITERATIONS: usize = 50;
pub const MIN_SAMPLES: usize = 200;
pub const MAX_SAMPLES: usize = 10_000;
pub const MIN_DURATION: Duration = Duration::from_secs(5);
pub const MAX_DURATION: Duration = Duration::from_secs(60);

pub fn nearest_rank(samples: &[u64], percentile: u64) -> u64 {
    assert!(!samples.is_empty(), "percentile requires samples");
    assert!((1..=100).contains(&percentile), "percentile is 1..=100");
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let rank = (percentile as usize * ordered.len()).div_ceil(100);
    ordered[rank - 1]
}

pub fn measure<S, O, E>(mut synchronize: S, mut operation: O) -> Result<(Vec<u64>, u64), E>
where
    S: FnMut() -> Result<(), E>,
    O: FnMut() -> Result<(), E>,
{
    for _ in 0..WARMUP_ITERATIONS {
        synchronize()?;
        operation()?;
        synchronize()?;
    }
    let measurement_started = Instant::now();
    let mut samples = Vec::with_capacity(MIN_SAMPLES);
    loop {
        synchronize()?;
        let started = Instant::now();
        operation()?;
        synchronize()?;
        samples.push(duration_ns(started.elapsed()).max(1));
        let elapsed = measurement_started.elapsed();
        if (samples.len() >= MIN_SAMPLES && elapsed >= MIN_DURATION)
            || samples.len() >= MAX_SAMPLES
            || elapsed >= MAX_DURATION
        {
            return Ok((samples, duration_ns(elapsed)));
        }
    }
}

pub fn validate_measurement_window(samples: &[u64], elapsed_ns: u64) -> Result<(), String> {
    if samples.len() < MIN_SAMPLES {
        return Err(format!(
            "timing series has {} samples; at least {MIN_SAMPLES} are required",
            samples.len()
        ));
    }
    if samples.len() > MAX_SAMPLES {
        return Err(format!(
            "timing series has {} samples; at most {MAX_SAMPLES} are allowed",
            samples.len()
        ));
    }
    if elapsed_ns > duration_ns(MAX_DURATION) {
        return Err("timing series exceeded the 60-second cap".to_owned());
    }
    if elapsed_ns < duration_ns(MIN_DURATION) && samples.len() != MAX_SAMPLES {
        return Err(
            "timing series ended before five seconds without reaching the 10,000-sample cap"
                .to_owned(),
        );
    }
    if samples.contains(&0) {
        return Err("timing series contains a zero-duration sample".to_owned());
    }
    Ok(())
}

pub fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub fn gflops(flop_count: u64, nanoseconds: u64) -> f64 {
    flop_count as f64 / nanoseconds.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_does_not_interpolate() {
        let samples = [5, 1, 4, 2, 3];
        assert_eq!(nearest_rank(&samples, 50), 3);
        assert_eq!(nearest_rank(&samples, 95), 5);
    }

    #[test]
    fn gflops_uses_integer_nanoseconds() {
        assert_eq!(gflops(2_000_000_000, 1_000_000_000), 2.0);
    }

    #[test]
    fn validates_each_measurement_window_independently() {
        assert!(validate_measurement_window(&vec![1; 200], 5_000_000_000).is_ok());
        assert!(validate_measurement_window(&vec![1; 10_000], 1_000_000).is_ok());
        assert!(validate_measurement_window(&vec![1; 200], 4_999_999_999).is_err());
        assert!(validate_measurement_window(&vec![1; 199], 5_000_000_000).is_err());
        assert!(validate_measurement_window(&vec![1; 200], 60_000_000_001).is_err());
    }
}
