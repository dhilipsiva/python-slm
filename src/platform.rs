use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostPlatform {
    WindowsX86_64,
    LinuxX86_64,
    MacosAppleSilicon,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AcceleratorProvider {
    Cuda,
    Rocm,
    Metal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Implemented,
    DeferredPostP16,
}

pub fn require_prototype_tuple(host: HostPlatform, provider: AcceleratorProvider) -> Result<()> {
    if host == HostPlatform::WindowsX86_64 && provider == AcceleratorProvider::Cuda {
        return Ok(());
    }
    Err(ProductError::gate(
        "DEFERRED_POST_P16",
        format!("{host:?}/{provider:?} is designed but deferred"),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_prototype_tuple_is_implemented() {
        assert!(
            require_prototype_tuple(HostPlatform::WindowsX86_64, AcceleratorProvider::Cuda).is_ok()
        );
        for tuple in [
            (HostPlatform::LinuxX86_64, AcceleratorProvider::Cuda),
            (HostPlatform::WindowsX86_64, AcceleratorProvider::Rocm),
            (HostPlatform::MacosAppleSilicon, AcceleratorProvider::Metal),
        ] {
            assert_eq!(
                require_prototype_tuple(tuple.0, tuple.1).unwrap_err().code,
                "DEFERRED_POST_P16"
            );
        }
    }
}
