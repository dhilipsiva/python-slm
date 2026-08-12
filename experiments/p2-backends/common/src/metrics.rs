use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    pub absolute: f64,
    pub relative: f64,
    pub relative_l2: Option<f64>,
    pub cosine: Option<f64>,
}

impl Tolerance {
    pub const FORWARD: Self = Self {
        absolute: 0.007_812_5,
        relative: 0.015_625,
        relative_l2: Some(0.015),
        cosine: None,
    };
    pub const GRADIENT: Self = Self {
        absolute: 0.007_812_5,
        relative: 0.031_25,
        relative_l2: Some(0.03),
        cosine: Some(0.999),
    };
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ElementMetrics {
    pub elements: u64,
    pub max_absolute_error: f64,
    pub max_relative_error: f64,
    pub relative_l2: f64,
    pub cosine_similarity: f64,
    pub envelope_violation_count: u64,
}

impl ElementMetrics {
    pub fn evaluate(actual: &[f64], reference: &[f64], tolerance: Tolerance) -> Self {
        assert_eq!(actual.len(), reference.len(), "metric shapes must match");
        let mut max_absolute_error = 0.0_f64;
        let mut max_relative_error = 0.0_f64;
        let mut error_squared = 0.0_f64;
        let mut reference_squared = 0.0_f64;
        let mut actual_squared = 0.0_f64;
        let mut dot = 0.0_f64;
        let mut envelope_violation_count = 0_u64;
        let mut saw_nonfinite = false;

        for (&actual, &reference) in actual.iter().zip(reference) {
            if !actual.is_finite() || !reference.is_finite() {
                saw_nonfinite = true;
                envelope_violation_count += 1;
                continue;
            }
            let absolute_error = (actual - reference).abs();
            max_absolute_error = max_absolute_error.max(absolute_error);
            max_relative_error =
                max_relative_error.max(absolute_error / reference.abs().max(1.0e-6));
            error_squared += absolute_error * absolute_error;
            reference_squared += reference * reference;
            actual_squared += actual * actual;
            dot += actual * reference;
            let allowed = tolerance.absolute + tolerance.relative * reference.abs();
            if absolute_error > allowed {
                envelope_violation_count += 1;
            }
        }

        let reference_norm = reference_squared.sqrt();
        let actual_norm = actual_squared.sqrt();
        let mut relative_l2 = error_squared.sqrt() / reference_norm.max(1.0e-12);
        let mut cosine_similarity = match (actual_norm, reference_norm) {
            (0.0, 0.0) => 1.0,
            (0.0, _) | (_, 0.0) => 0.0,
            _ => (dot / (actual_norm * reference_norm)).clamp(-1.0, 1.0),
        };
        if saw_nonfinite {
            max_absolute_error = f64::MAX;
            max_relative_error = f64::MAX;
            relative_l2 = f64::MAX;
            cosine_similarity = 0.0;
        }
        Self {
            elements: actual.len() as u64,
            max_absolute_error,
            max_relative_error,
            relative_l2,
            cosine_similarity,
            envelope_violation_count,
        }
    }

    pub fn passes(&self, tolerance: Tolerance) -> bool {
        self.envelope_violation_count == 0
            && tolerance
                .relative_l2
                .is_none_or(|limit| self.relative_l2 <= limit)
            && tolerance
                .cosine
                .is_none_or(|limit| self.cosine_similarity >= limit)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarMetrics {
    pub actual: f64,
    pub reference: f64,
    pub absolute_error: f64,
    pub allowed_error: f64,
    pub passed: bool,
}

impl ScalarMetrics {
    pub fn loss(actual: f64, reference: f64) -> Self {
        let finite = actual.is_finite() && reference.is_finite();
        let absolute_error = if finite {
            (actual - reference).abs()
        } else {
            f64::MAX
        };
        let allowed_error = 1.0e-5 + 0.01 * reference.abs();
        Self {
            actual: if actual.is_finite() { actual } else { f64::MAX },
            reference: if reference.is_finite() {
                reference
            } else {
                f64::MAX
            },
            absolute_error,
            allowed_error: if allowed_error.is_finite() {
                allowed_error
            } else {
                f64::MAX
            },
            passed: finite && absolute_error <= allowed_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_boundary_is_inclusive() {
        let reference = [1.0];
        let actual = [1.0 + Tolerance::FORWARD.absolute + Tolerance::FORWARD.relative];
        let metrics = ElementMetrics::evaluate(&actual, &reference, Tolerance::FORWARD);
        assert_eq!(metrics.envelope_violation_count, 0);
    }

    #[test]
    fn signed_zero_compares_equal() {
        let metrics = ElementMetrics::evaluate(&[-0.0], &[0.0], Tolerance::FORWARD);
        assert_eq!(metrics.max_absolute_error, 0.0);
        assert_eq!(metrics.cosine_similarity, 1.0);
    }

    #[test]
    fn nonfinite_values_fail_the_envelope() {
        let metrics = ElementMetrics::evaluate(&[f64::NAN], &[0.0], Tolerance::FORWARD);
        assert_eq!(metrics.envelope_violation_count, 1);
    }

    #[test]
    fn nonfinite_scalar_metrics_remain_strict_json_numbers() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let metric = ScalarMetrics::loss(value, 1.0);
            assert!(!metric.passed);
            assert_eq!(metric.actual, f64::MAX);
            let json = serde_json::to_string(&metric).unwrap();
            assert!(!json.contains("null"));
            assert!(serde_json::from_str::<ScalarMetrics>(&json).is_ok());
        }
    }
}
