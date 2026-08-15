use serde::Serialize;

pub mod accelerator;
pub mod oracle;
pub use accelerator::*;
pub use oracle::*;

pub const CANONICAL_MODEL_ID: &str = "gqa-135m-v1";
pub const REFERENCE_MODEL_ID: &str = "gqa-124m-ref-v1";
pub const CANONICAL_PARAMETER_COUNT: u64 = 135_285_504;
pub const REFERENCE_PARAMETER_COUNT: u64 = 124_668_672;
pub const COMPATIBILITY_ALLOCATION_BYTES: u64 = 2_952_790_016;
pub const STORED_TRAINING_PREFIX_IDS: u64 = 2_000_000_001;
pub const VALID_TRAINING_TARGETS: u64 = 2_000_000_000;
pub const TARGETS_PER_FULL_UPDATE: u64 = 65_536;
pub const FULL_UPDATES: u64 = 30_517;
pub const FINAL_UPDATE_TARGETS: u64 = 37_888;
pub const TOTAL_UPDATES: u64 = FULL_UPDATES + 1;
pub const ADMISSION_PROJECTION_SECONDS: u64 = 25_920;
pub const COMPLETION_SLA_SECONDS: u64 = 28_800;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelPreset {
    Gqa135mV1,
    Gqa124mRefV1,
}

impl ModelPreset {
    pub const fn identity(self) -> &'static str {
        match self {
            Self::Gqa135mV1 => CANONICAL_MODEL_ID,
            Self::Gqa124mRefV1 => REFERENCE_MODEL_ID,
        }
    }

    pub const fn parameter_count(self) -> u64 {
        match self {
            Self::Gqa135mV1 => CANONICAL_PARAMETER_COUNT,
            Self::Gqa124mRefV1 => REFERENCE_PARAMETER_COUNT,
        }
    }

    pub const fn canonical(self) -> bool {
        matches!(self, Self::Gqa135mV1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPlan {
    pub identity: &'static str,
    pub canonical: bool,
    pub parameters: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenPlan {
    pub stored_training_prefix_ids: u64,
    pub valid_training_targets: u64,
    pub boundary_exclusions: u64,
    pub targets_per_full_update: u64,
    pub full_updates: u64,
    pub final_update_targets: u64,
    pub total_updates: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePlan {
    pub compatibility_allocation_bytes: u64,
    pub admission_projection_seconds: u64,
    pub completion_sla_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub qualification_status: &'static str,
    pub model: ModelPlan,
    pub tokens: TokenPlan,
    pub runtime: RuntimePlan,
}

pub fn canonical_plan() -> PlanResult {
    PlanResult {
        schema: "python-slm-plan-result-v1",
        status: "PLANNED",
        qualification_status: "SKIPPED",
        model: ModelPlan {
            identity: CANONICAL_MODEL_ID,
            canonical: true,
            parameters: CANONICAL_PARAMETER_COUNT,
        },
        tokens: TokenPlan {
            stored_training_prefix_ids: STORED_TRAINING_PREFIX_IDS,
            valid_training_targets: VALID_TRAINING_TARGETS,
            boundary_exclusions: 0,
            targets_per_full_update: TARGETS_PER_FULL_UPDATE,
            full_updates: FULL_UPDATES,
            final_update_targets: FINAL_UPDATE_TARGETS,
            total_updates: TOTAL_UPDATES,
        },
        runtime: RuntimePlan {
            compatibility_allocation_bytes: COMPATIBILITY_ALLOCATION_BYTES,
            admission_projection_seconds: ADMISSION_PROJECTION_SECONDS,
            completion_sla_seconds: COMPLETION_SLA_SECONDS,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_arithmetic_is_exact() {
        assert_eq!(
            FULL_UPDATES * TARGETS_PER_FULL_UPDATE + FINAL_UPDATE_TARGETS,
            VALID_TRAINING_TARGETS
        );
        assert_eq!(STORED_TRAINING_PREFIX_IDS, VALID_TRAINING_TARGETS + 1);
        let plan = canonical_plan();
        assert_eq!(plan.model.parameters, CANONICAL_PARAMETER_COUNT);
        assert_eq!(plan.tokens.boundary_exclusions, 0);
        assert_eq!(plan.tokens.total_updates, 30_518);
        assert_eq!(plan.runtime.compatibility_allocation_bytes, 2_952_790_016);
        assert_eq!(plan.runtime.admission_projection_seconds, 25_920);
        assert_eq!(plan.runtime.completion_sla_seconds, 28_800);
    }

    #[test]
    fn reference_model_is_explicitly_noncanonical() {
        assert!(!ModelPreset::Gqa124mRefV1.canonical());
        assert_eq!(ModelPreset::Gqa124mRefV1.identity(), REFERENCE_MODEL_ID);
        assert_eq!(
            ModelPreset::Gqa124mRefV1.parameter_count(),
            REFERENCE_PARAMETER_COUNT
        );
        assert!(ModelPreset::Gqa135mV1.canonical());
    }
}
