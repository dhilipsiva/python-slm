use rust_llm_pretrain::platform::{
    AcceleratorProvider, HOST_DATA_ADAPTER_SCHEMA, HostPlatform, NATIVE_FILESYSTEM_SEMANTICS,
    PORTABLE_ARTIFACT_SEMANTICS, current_host_data_adapter, require_prototype_tuple,
};
use std::path::Path;

#[test]
fn current_native_host_exposes_the_closed_portable_data_contract() {
    let adapter = current_host_data_adapter().unwrap();
    assert_eq!(adapter.schema, HOST_DATA_ADAPTER_SCHEMA);
    assert_eq!(adapter.target_triple, adapter.host.target_triple());
    assert_eq!(adapter.artifact_semantics, PORTABLE_ARTIFACT_SEMANTICS);
    assert_eq!(adapter.filesystem_semantics, NATIVE_FILESYSTEM_SEMANTICS);
    assert_eq!(adapter.qualification_status, "SKIPPED");

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    assert_eq!(adapter.host, HostPlatform::WindowsX86_64);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(adapter.host, HostPlatform::LinuxX86_64);
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(adapter.host, HostPlatform::MacosAppleSilicon);

    let value = serde_json::to_value(adapter).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 6);
    assert_eq!(value["qualification_status"], "SKIPPED");
}

#[test]
fn approved_host_identities_and_wire_values_are_stable() {
    let identities = [
        (
            HostPlatform::WindowsX86_64,
            "windows-x86-64",
            "x86_64-pc-windows-msvc",
        ),
        (
            HostPlatform::LinuxX86_64,
            "linux-x86-64",
            "x86_64-unknown-linux-gnu",
        ),
        (
            HostPlatform::MacosAppleSilicon,
            "macos-apple-silicon",
            "aarch64-apple-darwin",
        ),
    ];
    for (host, wire, triple) in identities {
        assert_eq!(serde_json::to_string(&host).unwrap(), format!("\"{wire}\""));
        assert_eq!(host.target_triple(), triple);
    }
}

#[test]
fn every_cpu_data_command_crosses_the_native_adapter_before_reading_config() {
    type DataCommand = fn(&Path) -> rust_llm_pretrain::error::Result<serde_json::Value>;
    let commands: [(&str, DataCommand); 5] = [
        ("CONFIG_READ_FAILED", rust_llm_pretrain::data::curate),
        (
            "TOKENIZER_CONFIG_READ_FAILED",
            rust_llm_pretrain::tokenizer::train_tokenizer,
        ),
        (
            "TOKEN_CONFIG_READ_FAILED",
            rust_llm_pretrain::storage::tokenize,
        ),
        (
            "CORPUS_CONFIG_READ_FAILED",
            rust_llm_pretrain::corpus::prepare,
        ),
        (
            "SPAN_CONFIG_READ_FAILED",
            rust_llm_pretrain::corpus::plan_spans,
        ),
    ];
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.json");
    for (expected, command) in commands {
        let error = command(&missing).unwrap_err();
        assert_eq!(error.code, expected);
        assert_ne!(error.code, "DEFERRED_POST_P16");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn portable_data_support_does_not_expand_accelerator_support() {
    assert!(
        require_prototype_tuple(HostPlatform::WindowsX86_64, AcceleratorProvider::Cuda).is_ok()
    );
    for (host, provider) in [
        (HostPlatform::LinuxX86_64, AcceleratorProvider::Cuda),
        (HostPlatform::LinuxX86_64, AcceleratorProvider::Rocm),
        (HostPlatform::MacosAppleSilicon, AcceleratorProvider::Metal),
    ] {
        assert_eq!(
            require_prototype_tuple(host, provider).unwrap_err().code,
            "DEFERRED_POST_P16"
        );
    }
}
