use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};

pub const PHASE_CONFIG_SCHEMA: &str = "python-slm-phase-config-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseConfigV1 {
    pub schema: String,
    pub profile: String,
}

impl PhaseConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != PHASE_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "CONFIG_SCHEMA_UNSUPPORTED",
                format!("configuration schema {} is not supported", self.schema),
            ));
        }
        if self.profile != PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                format!("profile {} is designed but not implemented", self.profile),
            ));
        }
        Ok(())
    }
}

pub fn parse_phase_config(bytes: &[u8]) -> Result<PhaseConfigV1> {
    let config = serde_json::from_slice::<PhaseConfigV1>(bytes).map_err(|error| {
        ProductError::usage(
            "CONFIG_INVALID",
            format!("configuration is not a closed {PHASE_CONFIG_SCHEMA} object: {error}"),
        )
    })?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_config_requires_every_field_and_rejects_unknown_fields() {
        let valid =
            br#"{"schema":"python-slm-phase-config-v1","profile":"prototype-windows-5090-v1"}"#;
        assert_eq!(
            parse_phase_config(valid).unwrap().profile,
            PROTOTYPE_PROFILE
        );
        assert_eq!(
            parse_phase_config(br#"{"schema":"python-slm-phase-config-v1"}"#)
                .unwrap_err()
                .code,
            "CONFIG_INVALID"
        );
        assert_eq!(
            parse_phase_config(
                br#"{"schema":"python-slm-phase-config-v1","profile":"prototype-windows-5090-v1","extra":true}"#
            )
            .unwrap_err()
            .code,
            "CONFIG_INVALID"
        );
    }

    #[test]
    fn unsupported_schema_and_profile_are_typed() {
        let schema = br#"{"schema":"future","profile":"prototype-windows-5090-v1"}"#;
        assert_eq!(
            parse_phase_config(schema).unwrap_err().code,
            "CONFIG_SCHEMA_UNSUPPORTED"
        );
        let profile = br#"{"schema":"python-slm-phase-config-v1","profile":"linux-cuda"}"#;
        assert_eq!(
            parse_phase_config(profile).unwrap_err().code,
            "DEFERRED_POST_P16"
        );
    }
}
