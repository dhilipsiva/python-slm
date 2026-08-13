use crate::error::{IoContext, Result, XtaskError};
use crate::{hash, publication};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_INSPECTED_BINARY_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_MEMBERS: usize = 1_000_000;
const MAX_ZIP_MEMBERS: usize = 1_000_000;

const FORBIDDEN_BINARY_TOKENS: &[&[u8]] = &[
    b"python.dll",
    b"python3.dll",
    b"libpython",
    b"pythonapi",
    b"py_initialize",
    b"pyimport_",
    b"pyobject_",
    b"pyeval_",
    b"pyrun_",
    b"pygilstate_",
    b"__pycache__",
    b"site-packages",
    b".dist-info",
    b".egg-info",
    b"cudart",
    b"cublas",
    b"cudnn",
    b"nvcuda.dll",
    b"nvrtc",
    b"__cuda",
    b".nv_fatbin",
    b"cudalaunchkernel",
    b"cudamalloc",
    b"amdhip64",
    b"hiprtc",
    b"hipblas",
    b"hiplaunchkernel",
    b"rocblas",
    b"rocsolver",
    b"rocrand",
    b"metalperformanceshaders",
    b"mtlcreatesystemdefaultdevice",
    b"metal.framework",
    b".metallib",
    b"mpsgraph",
    b"libtorch",
    b"torch_cpu",
    b"torch_cuda",
    b"onnxruntime",
    b"libtensorflow",
    b"caffe2",
];

const PYTHON_EXTENSIONS: &[&str] = &["py", "pyc", "pyo", "pyd", "whl", "egg"];
const PROVIDER_NATIVE_EXTENSIONS: &[&str] = &["air", "cubin", "fatbin", "hsaco", "metallib", "ptx"];
const ARCHIVE_EXTENSIONS: &[&str] = &["a", "lib", "rlib"];
const COFF_EXTENSIONS: &[&str] = &["obj", "o"];

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetArtifactScan {
    pub command: String,
    pub file_count: usize,
    pub pe_file_count: usize,
    pub manifest_sha256: String,
    pub python_artifacts: Vec<String>,
    pub accelerator_artifacts: Vec<String>,
    pub native_ml_backend_artifacts: Vec<String>,
}

pub(crate) fn scan_target(root: &Path, command: &str) -> Result<TargetArtifactScan> {
    if !root.exists() {
        require_target_materialized(command, 0)?;
        return Ok(TargetArtifactScan {
            command: command.to_owned(),
            file_count: 0,
            pe_file_count: 0,
            manifest_sha256: hash::bytes(&[]),
            python_artifacts: Vec::new(),
            accelerator_artifacts: Vec::new(),
            native_ml_backend_artifacts: Vec::new(),
        });
    }
    publication::require_no_follow_tree(root)?;
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    require_target_materialized(command, files.len())?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut manifest = String::new();
    let mut pe_file_count = 0usize;
    for (relative, path, bytes) in &files {
        let lower_path = relative.to_ascii_lowercase();
        if forbidden_path(&lower_path) {
            return Err(XtaskError::gate(
                "P1A_FORBIDDEN_BUILD_ARTIFACT",
                format!("quality target contains forbidden artifact path {relative}"),
                "Remove Python, accelerator, and native-ML artifacts from the CPU build.",
            ));
        }
        let digest = hash::file(path)?;
        manifest.push_str(&digest);
        manifest.push_str("  ");
        manifest.push_str(&bytes.to_string());
        manifest.push_str("  ");
        manifest.push_str(relative);
        manifest.push('\n');
        let prefix = read_prefix(path)?;
        require_no_forbidden_magic(path, &prefix)?;
        if is_pe_candidate(path) || prefix.starts_with(b"MZ") {
            let imports = pe_imports(path).map_err(|error| artifact_error_context(path, error))?;
            pe_file_count += 1;
            if let Some(import) = imports
                .iter()
                .find(|import| forbidden_binary_name(import.as_bytes()))
            {
                return Err(XtaskError::gate(
                    "P1A_FORBIDDEN_BUILD_IMPORT",
                    format!("quality target PE artifact imports forbidden module {import}"),
                    "Remove Python, accelerator, and native-ML imports from the CPU build.",
                ));
            }
        } else if is_archive_candidate(path) || prefix.starts_with(b"!<arch>\n") {
            inspect_archive(path, parser_wrapper_archive(&lower_path))?;
        } else if is_coff_candidate(path) || looks_like_coff(&prefix) {
            inspect_coff_file(path)?;
        } else if is_zip_candidate(path) || looks_like_zip(&prefix) {
            inspect_zip_file(path)?;
        }
    }
    Ok(TargetArtifactScan {
        command: command.to_owned(),
        file_count: files.len(),
        pe_file_count,
        manifest_sha256: hash::bytes(manifest.as_bytes()),
        python_artifacts: Vec::new(),
        accelerator_artifacts: Vec::new(),
        native_ml_backend_artifacts: Vec::new(),
    })
}

fn require_target_materialized(command: &str, file_count: usize) -> Result<()> {
    if command != "fmt" && file_count == 0 {
        return Err(XtaskError::integrity(
            "P1A_QUALITY_TARGET_EMPTY",
            format!("the successful cargo {command} command produced no target artifacts"),
        ));
    }
    Ok(())
}

fn forbidden_path(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    if components.iter().any(|component| {
        let extension = component.rsplit_once('.').map(|(_, value)| value);
        extension.is_some_and(|value| PYTHON_EXTENSIONS.contains(&value))
            || extension.is_some_and(|value| PROVIDER_NATIVE_EXTENSIONS.contains(&value))
            || component.ends_with(".egg-info")
            || component.ends_with(".dist-info")
    }) {
        return true;
    }

    if components.iter().enumerate().any(|(index, component)| {
        component.contains("python")
            && *component != "python-slm"
            && !allowed_parser_component(&components, index)
    }) {
        return true;
    }

    components.iter().enumerate().any(|(index, component)| {
        if *component == "python-slm"
            || component.contains("python") && allowed_parser_component(&components, index)
        {
            return false;
        }
        component
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .any(forbidden_path_token)
    })
}

fn allowed_parser_component(components: &[&str], index: usize) -> bool {
    let component = components[index];
    if cargo_unit_component(component, "tree-sitter-python-") {
        return true;
    }
    if cargo_hashed_artifact(component, "tree_sitter_python-", &["d"])
        || cargo_hashed_artifact(component, "libtree_sitter_python-", &["rlib", "rmeta"])
    {
        return true;
    }

    let in_exact_unit = components
        .iter()
        .any(|value| cargo_unit_component(value, "tree-sitter-python-"));
    if in_exact_unit
        && matches!(
            component,
            "dep-lib-tree_sitter_python"
                | "lib-tree_sitter_python"
                | "lib-tree_sitter_python.json"
                | "tree-sitter-python.lib"
                | "libtree-sitter-python.a"
        )
    {
        return true;
    }
    false
}

fn cargo_unit_component(component: &str, prefix: &str) -> bool {
    component.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == 16 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn cargo_hashed_artifact(component: &str, prefix: &str, extensions: &[&str]) -> bool {
    let Some((stem, extension)) = component.rsplit_once('.') else {
        return false;
    };
    let Some(hash) = stem.strip_prefix(prefix) else {
        return false;
    };
    extensions.contains(&extension)
        && hash.len() == 16
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parser_wrapper_archive(path: &str) -> bool {
    if path.rsplit('/').next().is_some_and(|component| {
        cargo_hashed_artifact(component, "libtree_sitter_python-", &["rlib"])
            || cargo_hashed_artifact(component, "libtree_sitter-", &["rlib"])
    }) {
        return true;
    }
    let components = path.split('/').collect::<Vec<_>>();
    components.windows(4).any(|window| {
        window[0] == "build"
            && window[2] == "out"
            && ((cargo_unit_component(window[1], "tree-sitter-")
                && matches!(window[3], "tree-sitter.lib" | "libtree-sitter.a"))
                || (cargo_unit_component(window[1], "tree-sitter-python-")
                    && matches!(
                        window[3],
                        "tree-sitter-python.lib" | "libtree-sitter-python.a"
                    )))
    })
}

fn parser_wrapper_object_member(name: &str) -> bool {
    let Some((crate_id, codegen)) = name.split_once(".tree_sitter_python.") else {
        return false;
    };
    let Some(crate_hash) = crate_id.strip_prefix("tree_sitter_python-") else {
        return false;
    };
    let Some((unit_hash, tail)) = codegen.split_once("-cgu.") else {
        return false;
    };
    crate_hash.len() == 16
        && crate_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && unit_hash.len() == 16
        && unit_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && tail.strip_suffix(".rcgu.o").is_some_and(|index| {
            !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn generated_tree_sitter_object_member(name: &str) -> bool {
    let basename = name.rsplit('/').next().unwrap_or(name);
    let Some((hash, suffix)) = basename.split_once('-') else {
        return false;
    };
    hash.len() == 16
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(suffix, "lib.o" | "parser.o" | "scanner.o")
}

fn forbidden_path_token(token: &str) -> bool {
    let without_lib = token.strip_prefix("lib").unwrap_or(token);
    matches!(
        without_lib,
        "python"
            | "pypy"
            | "pip"
            | "cuda"
            | "cudnn"
            | "cublas"
            | "nvcc"
            | "nvidia"
            | "rocm"
            | "rocblas"
            | "hip"
            | "hipcc"
            | "hipblas"
            | "amdhip"
            | "torch"
            | "onnxruntime"
            | "tensorflow"
            | "metal"
            | "metallib"
            | "mps"
    ) || without_lib.starts_with("pip")
        && without_lib[3..].bytes().all(|byte| byte.is_ascii_digit())
        || without_lib.starts_with("pypy")
        || without_lib.starts_with("cpython")
        || without_lib.starts_with("pyvenv")
        || without_lib.starts_with("pytest")
        || without_lib.starts_with("cudart")
        || without_lib.starts_with("nvcuda")
        || without_lib.starts_with("amdhip64")
        || python_version_token(without_lib)
}

fn python_version_token(token: &str) -> bool {
    token.strip_prefix("python").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn read_prefix(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path).io_context(
        "P1A_ARTIFACT_SCAN_FAILED",
        "could not open a quality-target artifact for signature inspection",
    )?;
    // Provider-native text headers may follow a generated comment preamble. Keep this
    // bounded, but inspect enough of an otherwise opaque artifact to recognize PTX.
    let mut prefix = vec![0u8; 64 * 1024];
    let read = file.read(&mut prefix).io_context(
        "P1A_ARTIFACT_SCAN_FAILED",
        "could not read a quality-target artifact signature",
    )?;
    prefix.truncate(read);
    Ok(prefix)
}

fn require_no_forbidden_magic(path: &Path, prefix: &[u8]) -> Result<()> {
    let python_bytecode =
        prefix.len() >= 4 && prefix[0] != 0 && prefix[2] == 0x0d && prefix[3] == 0x0a;
    let python_shebang = prefix.starts_with(b"#!")
        && prefix
            .split(|byte| *byte == b'\n')
            .next()
            .is_some_and(forbidden_python_shebang);
    if python_bytecode || python_shebang || contains_provider_payload(prefix) {
        return Err(XtaskError::gate(
            "P1A_FORBIDDEN_BUILD_SIGNATURE",
            format!(
                "quality target contains a forbidden executable or provider signature in {}",
                path.display()
            ),
            "Remove Python, accelerator, and native-ML artifacts from the CPU build.",
        ));
    }
    Ok(())
}

fn contains_provider_payload(bytes: &[u8]) -> bool {
    contains_cuda_fatbin(bytes)
        || bytes.starts_with(b"MTLB")
        || provider_elf(bytes)
        || contains_ptx_program(bytes)
}

fn contains_cuda_fatbin(bytes: &[u8]) -> bool {
    for offset in 0..bytes.len().saturating_sub(3) {
        let magic = u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("four-byte window"),
        );
        if magic == 0x4662_43b1 {
            let version = bytes
                .get(offset + 4..offset + 8)
                .map(|value| u32::from_le_bytes(value.try_into().expect("four bytes")));
            if version == Some(1) && bytes.len().saturating_sub(offset) >= 24 {
                return true;
            }
        } else if magic == 0xba55_ed50 {
            let Some(header) = bytes.get(offset + 4..offset + 16) else {
                continue;
            };
            let version = u16::from_le_bytes(header[..2].try_into().expect("two bytes"));
            let header_size = u16::from_le_bytes(header[2..4].try_into().expect("two bytes"));
            let payload_size = u64::from_le_bytes(header[4..12].try_into().expect("eight bytes"));
            if (1..=16).contains(&version)
                && (16..=4096).contains(&header_size)
                && payload_size != 0
            {
                return true;
            }
        }
    }
    false
}

fn provider_elf(bytes: &[u8]) -> bool {
    if bytes.get(..4) != Some(b"\x7fELF") {
        return false;
    }
    let little_endian = bytes.get(5) == Some(&1);
    let machine = bytes.get(18..20);
    little_endian
        && machine.is_some_and(|value| {
            let machine = u16::from_le_bytes([value[0], value[1]]);
            // EM_CUDA and EM_AMDGPU.
            matches!(machine, 190 | 224)
        })
}

fn contains_ptx_program(bytes: &[u8]) -> bool {
    let mut search = 0usize;
    while let Some(relative) = bytes.get(search..).and_then(|tail| {
        tail.windows(b".version".len())
            .position(|part| part == b".version")
    }) {
        let version = search + relative;
        if ptx_directive_at_line(bytes, version, b".version")
            && parse_ptx_version_line(bytes, version + b".version".len())
            && contains_ptx_target_after(bytes, version)
        {
            return true;
        }
        search = version + 1;
    }
    false
}

fn ptx_directive_at_line(bytes: &[u8], offset: usize, directive: &[u8]) -> bool {
    if bytes.get(offset..offset.saturating_add(directive.len())) != Some(directive) {
        return false;
    }
    let line_start = bytes[..offset]
        .iter()
        .rposition(|byte| matches!(byte, b'\n' | b'\r' | 0))
        .map_or(0, |index| index + 1);
    if only_ptx_trivia(&bytes[line_start..offset]) {
        return true;
    }
    let payload_start = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == 0)
        .map_or(0, |index| index + 1);
    only_ptx_trivia(&bytes[payload_start..offset])
}

fn only_ptx_trivia(bytes: &[u8]) -> bool {
    let mut offset = usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3;
    while offset < bytes.len() {
        if bytes[offset].is_ascii_whitespace() {
            offset += 1;
        } else if bytes.get(offset..offset.saturating_add(2)) == Some(b"//") {
            offset += 2;
            while bytes.get(offset).is_some_and(|byte| *byte != b'\n') {
                offset += 1;
            }
        } else if bytes.get(offset..offset.saturating_add(2)) == Some(b"/*") {
            let Some(end) = bytes[offset + 2..]
                .windows(2)
                .position(|part| part == b"*/")
            else {
                return false;
            };
            offset += 2 + end + 2;
        } else {
            return false;
        }
    }
    true
}

fn parse_ptx_version_line(bytes: &[u8], mut offset: usize) -> bool {
    if !consume_horizontal_space(bytes, &mut offset) || !consume_digits(bytes, &mut offset) {
        return false;
    }
    if bytes.get(offset) != Some(&b'.') {
        return false;
    }
    offset += 1;
    if !consume_digits(bytes, &mut offset) {
        return false;
    }
    consume_ptx_line_end(bytes, offset)
}

fn contains_ptx_target_after(bytes: &[u8], version: usize) -> bool {
    let end = bytes.len().min(version.saturating_add(64 * 1024));
    let mut search = version + b".version".len();
    while let Some(relative) = bytes.get(search..end).and_then(|tail| {
        tail.windows(b".target".len())
            .position(|part| part == b".target")
    }) {
        let target = search + relative;
        if ptx_directive_at_line(bytes, target, b".target")
            && parse_ptx_target_line(bytes, target + b".target".len())
        {
            return true;
        }
        search = target + 1;
    }
    false
}

fn parse_ptx_target_line(bytes: &[u8], mut offset: usize) -> bool {
    if !consume_horizontal_space(bytes, &mut offset) {
        return false;
    }
    let target_start = offset;
    while bytes
        .get(offset)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        offset += 1;
    }
    let target = &bytes[target_start..offset];
    if !(target
        .strip_prefix(b"sm_")
        .or_else(|| target.strip_prefix(b"compute_"))
        .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit)))
    {
        return false;
    }
    consume_ptx_line_end(bytes, offset) || bytes.get(offset) == Some(&b',')
}

fn consume_horizontal_space(bytes: &[u8], offset: &mut usize) -> bool {
    let start = *offset;
    while bytes
        .get(*offset)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        *offset += 1;
    }
    *offset != start
}

fn consume_digits(bytes: &[u8], offset: &mut usize) -> bool {
    let start = *offset;
    while bytes.get(*offset).is_some_and(u8::is_ascii_digit) {
        *offset += 1;
    }
    *offset != start
}

fn consume_ptx_line_end(bytes: &[u8], mut offset: usize) -> bool {
    while bytes
        .get(offset)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        offset += 1;
    }
    bytes
        .get(offset)
        .is_none_or(|byte| matches!(byte, b'\r' | b'\n' | 0))
        || bytes.get(offset..offset.saturating_add(2)) == Some(b"//")
}

fn forbidden_python_shebang(line: &[u8]) -> bool {
    let lower = line
        .iter()
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    lower
        .windows(b"python".len())
        .any(|value| value == b"python")
        || lower.windows(b"pypy".len()).any(|value| value == b"pypy")
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn is_archive_candidate(path: &Path) -> bool {
    extension(path).is_some_and(|value| ARCHIVE_EXTENSIONS.contains(&value.as_str()))
}

fn is_coff_candidate(path: &Path) -> bool {
    extension(path).is_some_and(|value| COFF_EXTENSIONS.contains(&value.as_str()))
}

fn is_zip_candidate(path: &Path) -> bool {
    extension(path).is_some_and(|value| matches!(value.as_str(), "zip" | "whl" | "egg"))
}

fn looks_like_zip(prefix: &[u8]) -> bool {
    prefix.starts_with(b"PK\x03\x04")
        || prefix.starts_with(b"PK\x05\x06")
        || prefix.starts_with(b"PK\x07\x08")
}

fn looks_like_coff(prefix: &[u8]) -> bool {
    if prefix.len() < 20 {
        return false;
    }
    let machine = u16::from_le_bytes([prefix[0], prefix[1]]);
    matches!(machine, 0x014c | 0x8664 | 0xaa64) || (machine == 0 && prefix[2..4] == [0xff, 0xff])
}

fn read_bounded_binary(path: &Path, description: &'static str) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).io_context(
        "P1A_ARTIFACT_SCAN_FAILED",
        "could not inspect a quality-target binary artifact",
    )?;
    if metadata.len() > MAX_INSPECTED_BINARY_BYTES {
        return Err(XtaskError::integrity(
            "P1A_ARTIFACT_SCAN_TOO_LARGE",
            format!("{description} exceeds the bounded artifact parser size"),
        ));
    }
    fs::read(path).io_context(
        "P1A_ARTIFACT_SCAN_FAILED",
        "could not read a quality-target binary artifact",
    )
}

fn forbidden_binary_name(bytes: &[u8]) -> bool {
    let lower = bytes
        .iter()
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    FORBIDDEN_BINARY_TOKENS
        .iter()
        .any(|token| lower.windows(token.len()).any(|value| value == *token))
        || contains_versioned_python_library(&lower)
}

fn contains_versioned_python_library(bytes: &[u8]) -> bool {
    for start in 0..bytes.len() {
        if !bytes[start..].starts_with(b"python") {
            continue;
        }
        let mut cursor = start + b"python".len();
        let digit_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"_d") {
            cursor += 2;
        }
        if cursor > digit_start
            && (bytes.get(cursor..cursor + 4) == Some(b".dll")
                || bytes.get(cursor..cursor + 4) == Some(b".lib"))
        {
            return true;
        }
    }
    false
}

fn inspect_archive(path: &Path, parser_archive: bool) -> Result<()> {
    let bytes = read_bounded_binary(path, "quality-target archive")?;
    inspect_archive_bytes(&bytes, 0, parser_archive)
        .map_err(|error| artifact_error_context(path, error))
}

fn inspect_archive_bytes(bytes: &[u8], depth: usize, parser_archive: bool) -> Result<()> {
    if depth > 2 || !bytes.starts_with(b"!<arch>\n") {
        return Err(XtaskError::integrity(
            "P1A_ARCHIVE_HEADER_INVALID",
            "quality-target archive lacks a valid ar header",
        ));
    }
    let mut offset = 8usize;
    let mut long_names: Option<&[u8]> = None;
    let mut member_count = 0usize;
    while offset < bytes.len() {
        member_count += 1;
        if member_count > MAX_ARCHIVE_MEMBERS {
            return Err(XtaskError::integrity(
                "P1A_ARCHIVE_MEMBER_LIMIT_EXCEEDED",
                "quality-target archive exceeds the closed member limit",
            ));
        }
        let header = bytes
            .get(offset..offset.saturating_add(60))
            .ok_or_else(archive_bounds_error)?;
        if &header[58..60] != b"`\n" {
            return Err(XtaskError::integrity(
                "P1A_ARCHIVE_MEMBER_HEADER_INVALID",
                "quality-target archive contains a malformed member header",
            ));
        }
        let size = parse_ascii_decimal(&header[48..58], "archive member size")?;
        let data_start = offset.checked_add(60).ok_or_else(archive_bounds_error)?;
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(archive_bounds_error)?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(archive_bounds_error)?;
        let raw_name = std::str::from_utf8(&header[..16]).map_err(|_| {
            XtaskError::integrity(
                "P1A_ARCHIVE_MEMBER_NAME_INVALID",
                "quality-target archive member name is not UTF-8",
            )
        })?;
        let (name, payload, special) = resolve_archive_member(raw_name, data, long_names)?;
        if name == "//" {
            long_names = Some(payload);
        } else if !special {
            let normalized = normalize_archive_member_name(&name)?;
            let parser_member = parser_archive
                && (parser_wrapper_object_member(&normalized)
                    || generated_tree_sitter_object_member(&normalized));
            if forbidden_path(&normalized) && !parser_member {
                return Err(XtaskError::gate(
                    "P1A_FORBIDDEN_ARCHIVE_MEMBER",
                    format!("quality-target archive contains forbidden member {name}"),
                    "Remove Python, accelerator, and native-ML members from CPU archives.",
                ));
            }
            inspect_archive_payload(&normalized, payload, depth, parser_archive).map_err(
                |mut error| {
                    let member = normalized.rsplit('/').next().unwrap_or("<invalid>");
                    error.message = format!("archive member {member}: {}", error.message);
                    error
                },
            )?;
        }
        offset = data_end
            .checked_add(size % 2)
            .ok_or_else(archive_bounds_error)?;
        if offset > bytes.len() {
            return Err(archive_bounds_error());
        }
    }
    if offset != bytes.len() {
        return Err(archive_bounds_error());
    }
    Ok(())
}

fn normalize_archive_member_name(name: &str) -> Result<String> {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty()
        || normalized
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(unsafe_archive_member_name(name));
    }

    let relative = if windows_drive_rooted(&normalized) {
        &normalized[3..]
    } else {
        if normalized.starts_with('/') || has_windows_drive_prefix(&normalized) {
            return Err(unsafe_archive_member_name(name));
        }
        normalized.as_str()
    };
    if relative.is_empty()
        || relative
            .split('/')
            .any(|component| matches!(component, "" | "." | "..") || component.contains(':'))
    {
        return Err(unsafe_archive_member_name(name));
    }
    Ok(normalized)
}

fn windows_drive_rooted(path: &str) -> bool {
    has_windows_drive_prefix(path) && path.as_bytes().get(2) == Some(&b'/')
}

fn has_windows_drive_prefix(path: &str) -> bool {
    path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

fn unsafe_archive_member_name(name: &str) -> XtaskError {
    XtaskError::integrity(
        "P1A_ARCHIVE_MEMBER_NAME_INVALID",
        format!("quality-target archive contains an unsafe member path {name}"),
    )
}

fn resolve_archive_member<'a>(
    raw_name: &str,
    data: &'a [u8],
    long_names: Option<&'a [u8]>,
) -> Result<(String, &'a [u8], bool)> {
    let field = raw_name.trim_end();
    if field == "/" || field == "/SYM64/" || field.starts_with("__.SYMDEF") {
        return Ok((field.to_owned(), data, true));
    }
    if field == "//" {
        return Ok((field.to_owned(), data, false));
    }
    if let Some(length) = field.strip_prefix("#1/") {
        let length = parse_decimal_text(length, "BSD archive member-name length")?;
        let name_bytes = data.get(..length).ok_or_else(archive_bounds_error)?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| {
            XtaskError::integrity(
                "P1A_ARCHIVE_MEMBER_NAME_INVALID",
                "quality-target BSD archive member name is not UTF-8",
            )
        })?;
        return Ok((name.to_owned(), &data[length..], false));
    }
    if let Some(index) = field.strip_prefix('/') {
        let index = parse_decimal_text(index, "GNU archive long-name offset")?;
        let table = long_names.ok_or_else(|| {
            XtaskError::integrity(
                "P1A_ARCHIVE_LONG_NAME_INVALID",
                "quality-target archive references a missing long-name table",
            )
        })?;
        let tail = table.get(index..).ok_or_else(archive_bounds_error)?;
        let end = tail
            .windows(2)
            .position(|value| value == b"/\n")
            .or_else(|| tail.iter().position(|byte| *byte == 0))
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_ARCHIVE_LONG_NAME_INVALID",
                    "quality-target archive long name is not terminated",
                )
            })?;
        let name = std::str::from_utf8(&tail[..end]).map_err(|_| {
            XtaskError::integrity(
                "P1A_ARCHIVE_MEMBER_NAME_INVALID",
                "quality-target archive long member name is not UTF-8",
            )
        })?;
        return Ok((name.to_owned(), data, false));
    }
    Ok((field.trim_end_matches('/').to_owned(), data, false))
}

fn inspect_archive_payload(
    name: &str,
    payload: &[u8],
    depth: usize,
    parser_archive: bool,
) -> Result<()> {
    if payload.starts_with(b"!<arch>\n") {
        return inspect_archive_bytes(payload, depth + 1, parser_archive);
    }
    if name == "lib.rmeta" {
        return require_no_forbidden_magic(Path::new(name), payload);
    }
    let extension = name.rsplit_once('.').map(|(_, value)| value);
    if looks_like_coff(payload) || extension.is_some_and(|value| matches!(value, "obj" | "o")) {
        validate_coff(payload)?;
    } else if payload.starts_with(b"MZ") {
        let imports = parse_pe_imports(payload)?;
        if imports
            .iter()
            .any(|import| forbidden_binary_name(import.as_bytes()))
        {
            return Err(forbidden_binary_signature_error());
        }
    } else {
        require_no_forbidden_magic(Path::new(name), payload)?;
    }
    Ok(())
}

fn parse_ascii_decimal(bytes: &[u8], label: &str) -> Result<usize> {
    let value = std::str::from_utf8(bytes).map_err(|_| {
        XtaskError::integrity(
            "P1A_ARCHIVE_MEMBER_HEADER_INVALID",
            format!("quality-target {label} is not ASCII"),
        )
    })?;
    parse_decimal_text(value.trim(), label)
}

fn parse_decimal_text(value: &str, label: &str) -> Result<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(XtaskError::integrity(
            "P1A_ARCHIVE_MEMBER_HEADER_INVALID",
            format!("quality-target {label} is not a canonical decimal"),
        ));
    }
    value.parse::<usize>().map_err(|_| {
        XtaskError::integrity(
            "P1A_ARCHIVE_MEMBER_HEADER_INVALID",
            format!("quality-target {label} exceeds the parser range"),
        )
    })
}

fn archive_bounds_error() -> XtaskError {
    XtaskError::integrity(
        "P1A_ARCHIVE_BOUNDS_INVALID",
        "quality-target archive member exceeds file bounds",
    )
}

fn inspect_coff_file(path: &Path) -> Result<()> {
    let bytes = read_bounded_binary(path, "quality-target COFF object")?;
    validate_coff(&bytes).map_err(|error| artifact_error_context(path, error))
}

fn validate_coff(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 20 {
        return Err(coff_bounds_error());
    }
    if bytes[..4] == [0, 0, 0xff, 0xff] {
        return validate_anonymous_coff(bytes);
    }
    let machine = u16::from_le_bytes([bytes[0], bytes[1]]);
    if !matches!(machine, 0x014c | 0x8664 | 0xaa64) {
        return Err(XtaskError::integrity(
            "P1A_COFF_HEADER_INVALID",
            "quality-target object has an unsupported COFF machine",
        ));
    }
    let sections = read_u16(bytes, 2)? as usize;
    if sections == 0 || sections > 65_279 {
        return Err(XtaskError::integrity(
            "P1A_COFF_HEADER_INVALID",
            "quality-target object has an invalid COFF section count",
        ));
    }
    let symbol_offset = read_u32(bytes, 8)? as usize;
    let symbol_count = read_u32(bytes, 12)? as usize;
    let optional_size = read_u16(bytes, 16)? as usize;
    let section_table = 20usize
        .checked_add(optional_size)
        .ok_or_else(coff_bounds_error)?;
    let section_end = section_table
        .checked_add(sections.checked_mul(40).ok_or_else(coff_bounds_error)?)
        .ok_or_else(coff_bounds_error)?;
    if section_end > bytes.len() {
        return Err(coff_bounds_error());
    }
    let strings = validate_coff_string_table(bytes, symbol_offset, symbol_count, 18)?;
    validate_coff_sections(bytes, section_table, sections, strings)?;
    validate_coff_symbols(bytes, symbol_offset, symbol_count, 18, strings)?;
    Ok(())
}

fn validate_anonymous_coff(bytes: &[u8]) -> Result<()> {
    const BIGOBJ_CLASS_ID: [u8; 16] = [
        0xc7, 0xa1, 0xba, 0xd1, 0xee, 0xba, 0xa9, 0x4b, 0xaf, 0x20, 0xfa, 0xf6, 0x6a, 0xa4, 0xdc,
        0xb8,
    ];
    if bytes.get(12..28) == Some(&BIGOBJ_CLASS_ID) {
        if bytes.len() < 56 {
            return Err(coff_bounds_error());
        }
        let sections = read_u32(bytes, 44)? as usize;
        let symbol_offset = read_u32(bytes, 48)? as usize;
        let symbol_count = read_u32(bytes, 52)? as usize;
        if sections == 0 || sections > 65_279 {
            return Err(XtaskError::integrity(
                "P1A_COFF_HEADER_INVALID",
                "quality-target bigobj has an invalid section count",
            ));
        }
        let section_end = 56usize
            .checked_add(sections.checked_mul(40).ok_or_else(coff_bounds_error)?)
            .ok_or_else(coff_bounds_error)?;
        if section_end > bytes.len() {
            return Err(coff_bounds_error());
        }
        let strings = validate_coff_string_table(bytes, symbol_offset, symbol_count, 20)?;
        validate_coff_sections(bytes, 56, sections, strings)?;
        validate_coff_symbols(bytes, symbol_offset, symbol_count, 20, strings)?;
        return Ok(());
    }

    let payload_size = read_u32(bytes, 12)? as usize;
    let end = 20usize
        .checked_add(payload_size)
        .ok_or_else(coff_bounds_error)?;
    if payload_size == 0 || end > bytes.len() {
        return Err(XtaskError::integrity(
            "P1A_COFF_IMPORT_HEADER_INVALID",
            "quality-target import object has a malformed payload length",
        ));
    }
    let payload = &bytes[20..end];
    let first_end = payload.iter().position(|byte| *byte == 0).ok_or_else(|| {
        XtaskError::integrity(
            "P1A_COFF_IMPORT_HEADER_INVALID",
            "quality-target import object lacks a symbol terminator",
        )
    })?;
    let library = payload
        .get(first_end + 1..)
        .and_then(|tail| tail.split(|byte| *byte == 0).next())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_COFF_IMPORT_HEADER_INVALID",
                "quality-target import object lacks a library name",
            )
        })?;
    let symbol = &payload[..first_end];
    if forbidden_binary_name(library) || forbidden_symbol_name(symbol, true) {
        return Err(forbidden_binary_signature_error());
    }
    Ok(())
}

fn validate_coff_string_table(
    bytes: &[u8],
    symbol_offset: usize,
    symbol_count: usize,
    record_size: usize,
) -> Result<Option<&[u8]>> {
    if (symbol_offset == 0) != (symbol_count == 0) {
        return Err(XtaskError::integrity(
            "P1A_COFF_SYMBOL_TABLE_INVALID",
            "quality-target object has an incomplete COFF symbol-table reference",
        ));
    }
    if symbol_count == 0 {
        return Ok(None);
    }
    let symbol_bytes = symbol_count
        .checked_mul(record_size)
        .ok_or_else(coff_bounds_error)?;
    let string_offset = symbol_offset
        .checked_add(symbol_bytes)
        .ok_or_else(coff_bounds_error)?;
    let string_size = read_u32(bytes, string_offset)? as usize;
    let strings = bytes
        .get(string_offset..string_offset.saturating_add(string_size))
        .filter(|value| string_size >= 4 && value.len() == string_size)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_COFF_STRING_TABLE_INVALID",
                "quality-target object has a malformed COFF string table",
            )
        })?;
    Ok(Some(strings))
}

fn validate_coff_sections(
    bytes: &[u8],
    section_table: usize,
    sections: usize,
    strings: Option<&[u8]>,
) -> Result<()> {
    for index in 0..sections {
        let section = section_table + index * 40;
        let name = resolve_coff_name(&bytes[section..section + 8], strings, true)?;
        if forbidden_section_name(name) {
            return Err(forbidden_binary_evidence_error("COFF section", name));
        }
        let raw_size = read_u32(bytes, section + 16)?;
        let raw_offset = read_u32(bytes, section + 20)?;
        let characteristics = read_u32(bytes, section + 36)?;
        let payload = if raw_size != 0 && raw_offset == 0 {
            if characteristics & 0x0000_0080 == 0 {
                return Err(coff_bounds_error());
            }
            &[][..]
        } else {
            bounded_region(bytes, raw_offset, raw_size)?
        };
        if contains_provider_payload(payload) {
            return Err(forbidden_binary_evidence_error(
                "provider payload in COFF section",
                name,
            ));
        }
        let relocation_offset = read_u32(bytes, section + 24)?;
        let relocation_count = read_u16(bytes, section + 32)? as u32;
        require_bounded_region(
            bytes,
            relocation_offset,
            relocation_count
                .checked_mul(10)
                .ok_or_else(coff_bounds_error)?,
        )?;
    }
    Ok(())
}

fn validate_coff_symbols(
    bytes: &[u8],
    symbol_offset: usize,
    symbol_count: usize,
    record_size: usize,
    strings: Option<&[u8]>,
) -> Result<()> {
    if symbol_count == 0 {
        return Ok(());
    }
    let auxiliary_offset = match record_size {
        18 => 17,
        20 => 19,
        _ => return Err(coff_bounds_error()),
    };
    let mut index = 0usize;
    while index < symbol_count {
        let offset = symbol_offset
            .checked_add(
                index
                    .checked_mul(record_size)
                    .ok_or_else(coff_bounds_error)?,
            )
            .ok_or_else(coff_bounds_error)?;
        let record = bytes
            .get(offset..offset.saturating_add(record_size))
            .ok_or_else(coff_bounds_error)?;
        let name = resolve_coff_name(&record[..8], strings, false)?;
        let undefined = match record_size {
            18 => i16::from_le_bytes(record[12..14].try_into().expect("two bytes")) == 0,
            20 => i32::from_le_bytes(record[12..16].try_into().expect("four bytes")) == 0,
            _ => return Err(coff_bounds_error()),
        };
        if forbidden_symbol_name(name, undefined) {
            return Err(forbidden_binary_evidence_error("COFF symbol", name));
        }
        let auxiliary = record[auxiliary_offset] as usize;
        index = index
            .checked_add(1 + auxiliary)
            .filter(|value| *value <= symbol_count)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_COFF_SYMBOL_TABLE_INVALID",
                    "quality-target object has an out-of-range auxiliary symbol count",
                )
            })?;
    }
    Ok(())
}

fn resolve_coff_name<'a>(
    field: &'a [u8],
    strings: Option<&'a [u8]>,
    section_name: bool,
) -> Result<&'a [u8]> {
    if field.len() != 8 {
        return Err(coff_bounds_error());
    }
    let string_offset = if field[..4] == [0, 0, 0, 0] {
        Some(u32::from_le_bytes(field[4..8].try_into().expect("four bytes")) as usize)
    } else if section_name && field[0] == b'/' {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(field.len());
        let digits = field.get(1..end).ok_or_else(coff_name_error)?;
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(coff_name_error());
        }
        let text = std::str::from_utf8(digits).map_err(|_| coff_name_error())?;
        Some(text.parse::<usize>().map_err(|_| coff_name_error())?)
    } else {
        None
    };
    if let Some(offset) = string_offset {
        let table = strings.ok_or_else(coff_name_error)?;
        if offset < 4 {
            return Err(coff_name_error());
        }
        let tail = table.get(offset..).ok_or_else(coff_name_error)?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(coff_name_error)?;
        if end == 0 {
            return Err(coff_name_error());
        }
        return Ok(&tail[..end]);
    }
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    if section_name && end == 0 {
        return Err(coff_name_error());
    }
    Ok(&field[..end])
}

fn coff_name_error() -> XtaskError {
    XtaskError::integrity(
        "P1A_COFF_NAME_INVALID",
        "quality-target object has a malformed COFF section or symbol name",
    )
}

fn forbidden_section_name(name: &[u8]) -> bool {
    let lower = name
        .iter()
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    forbidden_binary_name(&lower)
        || lower.starts_with(b".nv")
        || lower.starts_with(b".cuda")
        || lower.starts_with(b".hip")
        || lower.starts_with(b".amdgpu")
        || lower.starts_with(b".llvm.offloading")
        || lower.starts_with(b"__nv")
        || lower.starts_with(b"__metal")
}

fn forbidden_symbol_name(name: &[u8], undefined: bool) -> bool {
    let registration_symbol = name
        .windows(b"__fatbinwrap".len())
        .any(|part| part == b"__fatbinwrap")
        || name
            .windows(b"__device_stub__".len())
            .any(|part| part == b"__device_stub__")
        || name
            .windows(b"__cudaRegister".len())
            .any(|part| part == b"__cudaRegister");
    registration_symbol
        || (undefined && (external_provider_api_symbol(name) || forbidden_binary_name(name)))
}

fn external_provider_api_symbol(name: &[u8]) -> bool {
    // COFF external symbols may carry an x86 leading underscore and the import
    // thunk prefix. Provider-looking vocabulary inside a Rust/C++ mangled name
    // is source metadata, not evidence that the object references a provider ABI.
    let mut symbol = name;
    while symbol.first() == Some(&b'_') {
        symbol = &symbol[1..];
    }
    if let Some(import) = symbol.strip_prefix(b"imp_") {
        symbol = import;
        while symbol.first() == Some(&b'_') {
            symbol = &symbol[1..];
        }
    }
    [b"cuda".as_slice(), b"cu", b"hip", b"MTL", b"MPS", b"Py"]
        .iter()
        .any(|stem| {
            symbol.starts_with(stem)
                && symbol
                    .get(stem.len())
                    .is_some_and(|byte| byte.is_ascii_uppercase() || *byte == b'_')
        })
}

fn require_bounded_region(bytes: &[u8], offset: u32, size: u32) -> Result<()> {
    bounded_region(bytes, offset, size).map(|_| ())
}

fn bounded_region(bytes: &[u8], offset: u32, size: u32) -> Result<&[u8]> {
    if size == 0 {
        return Ok(&[]);
    }
    let offset = offset as usize;
    let size = size as usize;
    if offset == 0 {
        return Err(coff_bounds_error());
    }
    bytes
        .get(offset..offset.saturating_add(size))
        .filter(|value| value.len() == size)
        .ok_or_else(coff_bounds_error)
}

fn forbidden_binary_signature_error() -> XtaskError {
    XtaskError::gate(
        "P1A_FORBIDDEN_BUILD_SIGNATURE",
        "quality-target object or archive contains a forbidden runtime/provider signature",
        "Remove Python, accelerator, and native-ML artifacts from the CPU build.",
    )
}

fn forbidden_binary_evidence_error(kind: &str, name: &[u8]) -> XtaskError {
    let mut error = forbidden_binary_signature_error();
    error.message = format!(
        "quality-target object contains forbidden {kind} evidence {}",
        String::from_utf8_lossy(name)
    );
    error
}

fn coff_bounds_error() -> XtaskError {
    XtaskError::integrity(
        "P1A_COFF_BOUNDS_INVALID",
        "quality-target COFF structure exceeds file bounds",
    )
}

fn inspect_zip_file(path: &Path) -> Result<()> {
    let bytes = read_bounded_binary(path, "quality-target ZIP archive")?;
    inspect_zip_bytes(&bytes).map_err(|error| artifact_error_context(path, error))
}

fn artifact_error_context(path: &Path, mut error: XtaskError) -> XtaskError {
    error.message = format!("{}: {}", path.display(), error.message);
    error
}

fn inspect_zip_bytes(bytes: &[u8]) -> Result<()> {
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd = bytes[search_start..]
        .windows(4)
        .rposition(|value| value == b"PK\x05\x06")
        .map(|index| search_start + index)
        .ok_or_else(zip_structure_error)?;
    let fixed = bytes
        .get(eocd..eocd.saturating_add(22))
        .ok_or_else(zip_structure_error)?;
    let disk = u16::from_le_bytes([fixed[4], fixed[5]]);
    let central_disk = u16::from_le_bytes([fixed[6], fixed[7]]);
    let disk_entries = u16::from_le_bytes([fixed[8], fixed[9]]);
    let total_entries = u16::from_le_bytes([fixed[10], fixed[11]]);
    let central_size = u32::from_le_bytes(fixed[12..16].try_into().expect("four bytes"));
    let central_offset = u32::from_le_bytes(fixed[16..20].try_into().expect("four bytes"));
    let comment_size = u16::from_le_bytes([fixed[20], fixed[21]]) as usize;
    if disk != 0
        || central_disk != 0
        || disk_entries != total_entries
        || total_entries == u16::MAX
        || central_size == u32::MAX
        || central_offset == u32::MAX
        || total_entries as usize > MAX_ZIP_MEMBERS
        || eocd
            .checked_add(22 + comment_size)
            .is_none_or(|end| end != bytes.len())
    {
        return Err(zip_structure_error());
    }
    let central_offset = central_offset as usize;
    let central_end = central_offset
        .checked_add(central_size as usize)
        .ok_or_else(zip_structure_error)?;
    if central_end != eocd || central_end > bytes.len() {
        return Err(zip_structure_error());
    }
    let mut offset = central_offset;
    for _ in 0..total_entries {
        let header = bytes
            .get(offset..offset.saturating_add(46))
            .ok_or_else(zip_structure_error)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(zip_structure_error());
        }
        let name_size = u16::from_le_bytes([header[28], header[29]]) as usize;
        let extra_size = u16::from_le_bytes([header[30], header[31]]) as usize;
        let comment_size = u16::from_le_bytes([header[32], header[33]]) as usize;
        let disk_start = u16::from_le_bytes([header[34], header[35]]);
        let compressed_size = u32::from_le_bytes(header[20..24].try_into().expect("four bytes"));
        let uncompressed_size = u32::from_le_bytes(header[24..28].try_into().expect("four bytes"));
        let local_offset = u32::from_le_bytes(header[42..46].try_into().expect("four bytes"));
        if disk_start != 0
            || compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_offset == u32::MAX
        {
            return Err(zip_structure_error());
        }
        let entry_end = offset
            .checked_add(46)
            .and_then(|value| value.checked_add(name_size))
            .and_then(|value| value.checked_add(extra_size))
            .and_then(|value| value.checked_add(comment_size))
            .ok_or_else(zip_structure_error)?;
        let name_bytes = bytes
            .get(offset + 46..offset + 46 + name_size)
            .ok_or_else(zip_structure_error)?;
        let payload = require_matching_local_zip_entry(
            bytes,
            local_offset as usize,
            name_bytes,
            compressed_size as usize,
            central_offset,
        )?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| {
            XtaskError::integrity(
                "P1A_ZIP_MEMBER_NAME_INVALID",
                "quality-target ZIP member name is not UTF-8",
            )
        })?;
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        if normalized.starts_with('/')
            || normalized
                .split('/')
                .any(|component| matches!(component, "." | ".."))
        {
            return Err(XtaskError::integrity(
                "P1A_ZIP_MEMBER_NAME_INVALID",
                "quality-target ZIP contains an unsafe member path",
            ));
        }
        if forbidden_path(normalized.trim_end_matches('/')) {
            return Err(XtaskError::gate(
                "P1A_FORBIDDEN_ARCHIVE_MEMBER",
                format!("quality-target ZIP contains forbidden member {name}"),
                "Remove Python, accelerator, and native-ML members from CPU archives.",
            ));
        }
        if !normalized.ends_with('/') {
            let flags = u16::from_le_bytes([header[8], header[9]]);
            let compression = u16::from_le_bytes([header[10], header[11]]);
            if flags & 1 != 0 || (compression != 0 && uncompressed_size != 0) {
                return Err(XtaskError::integrity(
                    "P1A_ZIP_COMPRESSION_UNINSPECTABLE",
                    "quality-target ZIP member cannot be inspected without decoding",
                ));
            }
            if compression == 0 {
                if payload.len() != uncompressed_size as usize {
                    return Err(zip_structure_error());
                }
                require_no_forbidden_magic(Path::new(name), payload)?;
                if payload.starts_with(b"!<arch>\n") {
                    inspect_archive_bytes(payload, 0, false)?;
                } else if looks_like_coff(payload) {
                    validate_coff(payload)?;
                } else if payload.starts_with(b"MZ")
                    && parse_pe_imports(payload)?
                        .iter()
                        .any(|import| forbidden_binary_name(import.as_bytes()))
                {
                    return Err(forbidden_binary_signature_error());
                }
            }
        }
        offset = entry_end;
    }
    if offset != central_end {
        return Err(zip_structure_error());
    }
    Ok(())
}

fn require_matching_local_zip_entry<'a>(
    bytes: &'a [u8],
    offset: usize,
    expected_name: &[u8],
    compressed_size: usize,
    central_offset: usize,
) -> Result<&'a [u8]> {
    let header = bytes
        .get(offset..offset.saturating_add(30))
        .ok_or_else(zip_structure_error)?;
    if &header[..4] != b"PK\x03\x04" {
        return Err(zip_structure_error());
    }
    let name_size = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_size = u16::from_le_bytes([header[28], header[29]]) as usize;
    let name_start = offset.checked_add(30).ok_or_else(zip_structure_error)?;
    let name_end = name_start
        .checked_add(name_size)
        .ok_or_else(zip_structure_error)?;
    let data_start = name_end
        .checked_add(extra_size)
        .ok_or_else(zip_structure_error)?;
    let data_end = data_start
        .checked_add(compressed_size)
        .ok_or_else(zip_structure_error)?;
    if bytes.get(name_start..name_end) != Some(expected_name) || data_end > central_offset {
        return Err(zip_structure_error());
    }
    bytes
        .get(data_start..data_end)
        .ok_or_else(zip_structure_error)
}

fn zip_structure_error() -> XtaskError {
    XtaskError::integrity(
        "P1A_ZIP_STRUCTURE_INVALID",
        "quality-target ZIP archive has a malformed or unsupported structure",
    )
}

fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(String, PathBuf, u64)>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .io_context(
            "P1A_TARGET_SCAN_FAILED",
            "could not enumerate an isolated quality target",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_TARGET_SCAN_FAILED",
            "could not read an isolated quality target entry",
        )?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).io_context(
            "P1A_TARGET_SCAN_FAILED",
            "could not inspect an isolated quality target entry",
        )?;
        if metadata.is_dir() {
            collect_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| {
                    XtaskError::integrity(
                        "P1A_TARGET_SCAN_ESCAPED",
                        "quality target artifact escaped its owned root",
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            output.push((relative, path, metadata.len()));
        } else {
            return Err(XtaskError::integrity(
                "P1A_TARGET_SCAN_NONREGULAR",
                "quality target contains a nonregular entry",
            ));
        }
    }
    Ok(())
}

fn is_pe_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "dll" | "pyd"
            )
        })
}

fn pe_imports(path: &Path) -> Result<Vec<String>> {
    let metadata = fs::metadata(path).io_context(
        "P1A_PE_SCAN_FAILED",
        "could not inspect a quality-target PE artifact",
    )?;
    if metadata.len() > MAX_INSPECTED_BINARY_BYTES {
        return Err(XtaskError::integrity(
            "P1A_PE_SCAN_TOO_LARGE",
            "quality-target PE artifact exceeds the bounded parser size",
        ));
    }
    let bytes = fs::read(path).io_context(
        "P1A_PE_SCAN_FAILED",
        "could not read a quality-target PE artifact",
    )?;
    parse_pe_imports(&bytes)
}

fn parse_pe_imports(bytes: &[u8]) -> Result<Vec<String>> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err(XtaskError::integrity(
            "P1A_PE_HEADER_INVALID",
            "an .exe or .dll quality artifact lacks an MZ header",
        ));
    }
    let pe_offset = read_u32(bytes, 0x3c)? as usize;
    if bytes.get(pe_offset..pe_offset.saturating_add(4)) != Some(b"PE\0\0") {
        return Err(XtaskError::integrity(
            "P1A_PE_HEADER_INVALID",
            "a quality artifact lacks a PE signature",
        ));
    }
    let coff = pe_offset.checked_add(4).ok_or_else(pe_bounds_error)?;
    let sections = read_u16(bytes, coff + 2)? as usize;
    let optional_size = read_u16(bytes, coff + 16)? as usize;
    let optional = coff.checked_add(20).ok_or_else(pe_bounds_error)?;
    let magic = read_u16(bytes, optional)?;
    let (data_directory, number_of_directories_offset, minimum_optional_size) = match magic {
        0x10b => (optional + 96, optional + 92, 208usize),
        0x20b => (optional + 112, optional + 108, 224usize),
        _ => {
            return Err(XtaskError::integrity(
                "P1A_PE_OPTIONAL_HEADER_INVALID",
                "quality artifact has an unsupported PE optional-header magic",
            ));
        }
    };
    if optional_size < minimum_optional_size || read_u32(bytes, number_of_directories_offset)? < 14
    {
        return Err(XtaskError::integrity(
            "P1A_PE_OPTIONAL_HEADER_INVALID",
            "quality artifact optional header lacks the required import directories",
        ));
    }
    let section_table = optional
        .checked_add(optional_size)
        .ok_or_else(pe_bounds_error)?;
    let section_bytes = sections.checked_mul(40).ok_or_else(pe_bounds_error)?;
    if section_table
        .checked_add(section_bytes)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(pe_bounds_error());
    }
    validate_pe_sections(bytes, section_table, sections)?;
    let mut imports = Vec::new();
    let import_rva = read_u32(bytes, data_directory + 8)?;
    if import_rva != 0 {
        parse_import_descriptors(bytes, import_rva, section_table, sections, &mut imports)?;
    }
    let delay_rva = read_u32(bytes, data_directory + (13 * 8))?;
    if delay_rva != 0 {
        parse_delay_import_descriptors(bytes, delay_rva, section_table, sections, &mut imports)?;
    }
    imports.sort_by_key(|value| value.to_ascii_lowercase());
    imports.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    Ok(imports)
}

fn validate_pe_sections(bytes: &[u8], section_table: usize, sections: usize) -> Result<()> {
    for index in 0..sections {
        let section = section_table + index * 40;
        let name_field = bytes
            .get(section..section.saturating_add(8))
            .ok_or_else(pe_bounds_error)?;
        let name_end = name_field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_field.len());
        if name_end == 0 || forbidden_section_name(&name_field[..name_end]) {
            return Err(forbidden_binary_signature_error());
        }
        let raw_size = read_u32(bytes, section + 16)? as usize;
        let raw_offset = read_u32(bytes, section + 20)? as usize;
        let payload = if raw_size == 0 {
            &[][..]
        } else {
            bytes
                .get(raw_offset..raw_offset.saturating_add(raw_size))
                .filter(|value| raw_offset != 0 && value.len() == raw_size)
                .ok_or_else(pe_bounds_error)?
        };
        if contains_provider_payload(payload) {
            return Err(forbidden_binary_signature_error());
        }
    }
    Ok(())
}

fn parse_import_descriptors(
    bytes: &[u8],
    rva: u32,
    section_table: usize,
    sections: usize,
    imports: &mut Vec<String>,
) -> Result<()> {
    let mut offset = rva_to_offset(bytes, rva, section_table, sections)?;
    for _ in 0..4096 {
        let descriptor = bytes.get(offset..offset + 20).ok_or_else(pe_bounds_error)?;
        if descriptor.iter().all(|byte| *byte == 0) {
            return Ok(());
        }
        let name_rva = u32::from_le_bytes(descriptor[12..16].try_into().expect("four bytes"));
        imports.push(read_rva_string(bytes, name_rva, section_table, sections)?);
        offset = offset.checked_add(20).ok_or_else(pe_bounds_error)?;
    }
    Err(XtaskError::integrity(
        "P1A_PE_IMPORT_LIMIT_EXCEEDED",
        "quality artifact import table exceeds the closed descriptor limit",
    ))
}

fn parse_delay_import_descriptors(
    bytes: &[u8],
    rva: u32,
    section_table: usize,
    sections: usize,
    imports: &mut Vec<String>,
) -> Result<()> {
    let mut offset = rva_to_offset(bytes, rva, section_table, sections)?;
    for _ in 0..4096 {
        let descriptor = bytes.get(offset..offset + 32).ok_or_else(pe_bounds_error)?;
        if descriptor.iter().all(|byte| *byte == 0) {
            return Ok(());
        }
        let name_rva = u32::from_le_bytes(descriptor[4..8].try_into().expect("four bytes"));
        imports.push(read_rva_string(bytes, name_rva, section_table, sections)?);
        offset = offset.checked_add(32).ok_or_else(pe_bounds_error)?;
    }
    Err(XtaskError::integrity(
        "P1A_PE_IMPORT_LIMIT_EXCEEDED",
        "quality artifact delay-import table exceeds the closed descriptor limit",
    ))
}

fn read_rva_string(
    bytes: &[u8],
    rva: u32,
    section_table: usize,
    sections: usize,
) -> Result<String> {
    let offset = rva_to_offset(bytes, rva, section_table, sections)?;
    let tail = bytes.get(offset..).ok_or_else(pe_bounds_error)?;
    let end = tail
        .iter()
        .take(260)
        .position(|byte| *byte == 0)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_PE_IMPORT_NAME_INVALID",
                "quality artifact import name is absent or unbounded",
            )
        })?;
    let name = std::str::from_utf8(&tail[..end]).map_err(|_| {
        XtaskError::integrity(
            "P1A_PE_IMPORT_NAME_INVALID",
            "quality artifact import name is not UTF-8",
        )
    })?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(XtaskError::integrity(
            "P1A_PE_IMPORT_NAME_INVALID",
            "quality artifact import name is outside the closed alphabet",
        ));
    }
    Ok(name.to_owned())
}

fn rva_to_offset(bytes: &[u8], rva: u32, section_table: usize, sections: usize) -> Result<usize> {
    for index in 0..sections {
        let section = section_table + (index * 40);
        let virtual_size = read_u32(bytes, section + 8)?;
        let virtual_address = read_u32(bytes, section + 12)?;
        let raw_size = read_u32(bytes, section + 16)?;
        let raw_offset = read_u32(bytes, section + 20)?;
        let span = virtual_size.max(raw_size);
        if rva >= virtual_address && rva < virtual_address.saturating_add(span) {
            let offset = raw_offset
                .checked_add(rva - virtual_address)
                .ok_or_else(pe_bounds_error)? as usize;
            if offset < bytes.len() {
                return Ok(offset);
            }
        }
    }
    Err(XtaskError::integrity(
        "P1A_PE_RVA_INVALID",
        "quality artifact import RVA is outside every section",
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value: [u8; 2] = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(pe_bounds_error)?
        .try_into()
        .expect("slice length checked");
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(pe_bounds_error)?
        .try_into()
        .expect("slice length checked");
    Ok(u32::from_le_bytes(value))
}

fn pe_bounds_error() -> XtaskError {
    XtaskError::integrity(
        "P1A_PE_BOUNDS_INVALID",
        "quality artifact PE structure exceeds file bounds",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ar_member(name: &str, payload: &[u8]) -> Vec<u8> {
        assert!(name.len() <= 15);
        let mut header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            format!("{name}/"),
            0,
            0,
            0,
            0,
            payload.len()
        )
        .into_bytes();
        assert_eq!(header.len(), 60);
        header.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            header.push(b'\n');
        }
        header
    }

    fn bsd_ar_member(name: &str, payload: &[u8]) -> Vec<u8> {
        let member_size = name.len() + payload.len();
        let mut header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            format!("#1/{}", name.len()),
            0,
            0,
            0,
            0,
            member_size
        )
        .into_bytes();
        assert_eq!(header.len(), 60);
        header.extend_from_slice(name.as_bytes());
        header.extend_from_slice(payload);
        if member_size % 2 == 1 {
            header.push(b'\n');
        }
        header
    }

    fn minimal_coff(extra: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 60];
        bytes[0..2].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&1u16.to_le_bytes());
        bytes[20..28].copy_from_slice(b".text\0\0\0");
        bytes.extend_from_slice(extra);
        bytes
    }

    fn coff_with_section(name: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(name.len() <= 8);
        let mut bytes = minimal_coff(&[]);
        bytes[20..20 + name.len()].copy_from_slice(name);
        bytes[36..40].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[40..44].copy_from_slice(&60u32.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn coff_with_symbol(name: &[u8]) -> Vec<u8> {
        let mut bytes = minimal_coff(&[]);
        bytes[8..12].copy_from_slice(&60u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        let mut symbol = [0u8; 18];
        let mut strings = Vec::new();
        if name.len() <= 8 {
            symbol[..name.len()].copy_from_slice(name);
            strings.extend_from_slice(&4u32.to_le_bytes());
        } else {
            symbol[4..8].copy_from_slice(&4u32.to_le_bytes());
            let string_size = 4 + name.len() + 1;
            strings.extend_from_slice(&(string_size as u32).to_le_bytes());
            strings.extend_from_slice(name);
            strings.push(0);
        }
        bytes.extend_from_slice(&symbol);
        bytes.extend_from_slice(&strings);
        bytes
    }

    fn coff_with_long_section(name: &[u8]) -> Vec<u8> {
        let mut bytes = minimal_coff(&[]);
        bytes[20..28].copy_from_slice(b"/4\0\0\0\0\0\0");
        bytes[8..12].copy_from_slice(&60u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        let mut symbol = [0u8; 18];
        symbol[..8].copy_from_slice(b"ordinary");
        bytes.extend_from_slice(&symbol);
        bytes.extend_from_slice(&((4 + name.len() + 1) as u32).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.push(0);
        bytes
    }

    fn coff_import(symbol: &[u8], library: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(symbol);
        payload.push(0);
        payload.extend_from_slice(library);
        payload.push(0);
        let mut bytes = vec![0u8; 20];
        bytes[..4].copy_from_slice(&[0, 0, 0xff, 0xff]);
        bytes[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    #[test]
    fn rejects_non_pe_executable_and_forbidden_path() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("sample.exe"), b"not-pe").unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_PE_HEADER_INVALID"
        );
        fs::remove_file(temp.path().join("sample.exe")).unwrap();
        fs::write(temp.path().join("python-helper.txt"), b"x").unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_ARTIFACT"
        );
    }

    #[test]
    fn parser_exception_is_exact_and_does_not_mask_python_neighbors() {
        for allowed in [
            ".fingerprint/tree-sitter-python-0123456789abcdef/lib-tree_sitter_python.json",
            "build/tree-sitter-python-0123456789abcdef/out/tree-sitter-python.lib",
            "deps/libtree_sitter_python-0123456789abcdef.rlib",
        ] {
            assert!(
                !forbidden_path(allowed),
                "unexpected rejection of {allowed}"
            );
        }
        for rejected in [
            "tree-sitter-python-not-a-cargo-unit/python.exe",
            "build/tree-sitter-python-0123456789abcdef/out/python-helper.obj",
            "build/my-tree-sitter-python-0123456789abcdef/out/parser.lib",
            "deps/libtree_sitter_python-backdoor.rlib",
        ] {
            assert!(
                forbidden_path(rejected),
                "unexpected allowance of {rejected}"
            );
        }
        assert!(parser_wrapper_object_member(
            "tree_sitter_python-0123456789abcdef.tree_sitter_python.0123456789abcdef-cgu.0.rcgu.o"
        ));
        assert!(!parser_wrapper_object_member(
            "tree_sitter_python-0123456789abcdeg.tree_sitter_python.0123456789abcdef-cgu.0.rcgu.o"
        ));
    }

    #[test]
    fn rejects_python_extensions_magic_and_renamed_pe() {
        for name in [
            "input.py",
            "cache.pyc",
            "native.pyo",
            "native.pyd",
            "x.whl",
            "x.egg",
        ] {
            assert!(forbidden_path(name), "extension was not rejected: {name}");
        }

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("innocent.bin"), [0x42, 0x0d, 0x0d, 0x0a]).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
        fs::remove_file(temp.path().join("innocent.bin")).unwrap();
        fs::write(temp.path().join("renamed.dat"), b"MZnot-a-real-PE").unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_PE_BOUNDS_INVALID"
        );
    }

    #[test]
    fn rejects_forbidden_archive_member_and_renamed_binary_signature() {
        let temp = tempfile::tempdir().unwrap();
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member("cuda.obj", &minimal_coff(&[])));
        fs::write(temp.path().join("ordinary.lib"), archive).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_ARCHIVE_MEMBER"
        );

        fs::remove_file(temp.path().join("ordinary.lib")).unwrap();
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member(
            "ordinary.obj",
            &coff_with_symbol(b"cudaLaunchKernel"),
        ));
        fs::write(temp.path().join("ordinary.lib"), archive).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
    }

    #[test]
    fn accepts_minimal_legitimate_msvc_archive() {
        let temp = tempfile::tempdir().unwrap();
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member("ordinary.obj", &minimal_coff(b"ordinary_symbol")));
        fs::write(temp.path().join("ordinary.lib"), archive).unwrap();
        let result = scan_target(temp.path(), "test").unwrap();
        assert_eq!(result.file_count, 1);
    }

    #[test]
    fn accepts_ring_windows_absolute_member_metadata_without_weakening_name_checks() {
        let ring_member = concat!(
            r"C:\Users\dhilipsiva\.cargo\registry\src\",
            r"index.crates.io-1949cf8c6b5b557f\ring-0.17.14\pregenerated\",
            "sha256-x86_64-nasm.o"
        );
        assert_eq!(
            normalize_archive_member_name(ring_member).unwrap(),
            concat!(
                "c:/users/dhilipsiva/.cargo/registry/src/",
                "index.crates.io-1949cf8c6b5b557f/ring-0.17.14/pregenerated/",
                "sha256-x86_64-nasm.o"
            )
        );

        let temp = tempfile::tempdir().unwrap();
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(bsd_ar_member(
            ring_member,
            &minimal_coff(b"ordinary_symbol"),
        ));
        fs::write(temp.path().join("libring_core.a"), archive).unwrap();
        assert_eq!(scan_target(temp.path(), "test").unwrap().file_count, 1);

        for malicious in [
            r"C:\safe\..\ordinary.obj",
            r"C:relative\ordinary.obj",
            r"C:\safe\\ordinary.obj",
            r"\\server\share\ordinary.obj",
            r"\\?\C:\safe\ordinary.obj",
            "C:\\safe\\ordinary\0.obj",
            "/absolute/ordinary.obj",
        ] {
            assert_eq!(
                normalize_archive_member_name(malicious).unwrap_err().code,
                "P1A_ARCHIVE_MEMBER_NAME_INVALID",
                "unsafe archive member was accepted: {malicious:?}"
            );
        }

        let forbidden = normalize_archive_member_name(r"C:\safe\cuda.obj").unwrap();
        assert!(forbidden_path(&forbidden));
    }

    #[test]
    fn accepts_inert_rust_metadata_with_provider_vocabulary() {
        let temp = tempfile::tempdir().unwrap();
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member(
            "lib.rmeta",
            b"rust metadata describing inert cudart and cuda fields",
        ));
        fs::write(temp.path().join("ordinary.rlib"), archive).unwrap();
        let result = scan_target(temp.path(), "test").unwrap();
        assert_eq!(result.file_count, 1);
    }

    #[test]
    fn semantically_scans_rust_codegen_without_rejecting_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let inert = coff_with_section(
            b".rdata",
            b"diagnostic only: cudart64_12.dll cudaLaunchKernel .version and .target",
        );
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member("unit.rcgu.o", &inert));
        fs::write(temp.path().join("ordinary.rlib"), archive).unwrap();
        assert_eq!(scan_target(temp.path(), "test").unwrap().file_count, 1);

        fs::remove_file(temp.path().join("ordinary.rlib")).unwrap();
        let mut ptx_payload = Vec::new();
        for part in [
            &b"// generated provider program\n"[..],
            &b"\t\n  .ver"[..],
            &b"sion 8.8\n\t.tar"[..],
            &b"get sm_120\n.address_size 64\n"[..],
        ] {
            ptx_payload.extend_from_slice(part);
        }
        let ptx = coff_with_section(b".rdata", &ptx_payload);
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(ar_member("unit.rcgu.o", &ptx));
        fs::write(temp.path().join("ordinary.rlib"), archive).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
    }

    #[test]
    fn treats_provider_vocabulary_in_mangled_code_symbols_as_inert() {
        for inert in [
            b"_ZN18build_script_build21validate_cuda_version28_$u7b$$u7b$closure$u7d$$u7d$17h4b06dc927bf03e28E"
                .as_slice(),
            b"?validate_cuda_version@@YAXXZ",
        ] {
            validate_coff(&coff_with_symbol(inert)).unwrap();
        }

        for provider_reference in [
            b"cudaLaunchKernel".as_slice(),
            b"__imp_cudaMalloc",
            b"_hipLaunchKernel",
            b"__imp__Py_Initialize",
            b"MTLCreateSystemDefaultDevice",
        ] {
            assert_eq!(
                validate_coff(&coff_with_symbol(provider_reference))
                    .unwrap_err()
                    .code,
                "P1A_FORBIDDEN_BUILD_SIGNATURE",
                "provider ABI reference was accepted: {:?}",
                String::from_utf8_lossy(provider_reference)
            );
        }

        assert_eq!(
            validate_coff(&coff_with_long_section(b".nv_fatbin"))
                .unwrap_err()
                .code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
        assert_eq!(
            validate_coff(&coff_import(b"ordinary", b"cudart64_12.dll"))
                .unwrap_err()
                .code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
    }

    #[test]
    fn rejects_provider_sections_symbols_payloads_and_native_extensions() {
        let mut fatbin = Vec::new();
        fatbin.extend_from_slice(&0x4662_43b1u32.to_le_bytes());
        fatbin.extend_from_slice(&1u32.to_le_bytes());
        fatbin.resize(24, 0);
        for object in [
            coff_with_long_section(b".nv_fatbin"),
            coff_with_symbol(b"__cudaRegisterFatBinary"),
            coff_with_section(b".rdata", &fatbin),
            coff_import(b"ordinary", b"cudart64_12.dll"),
        ] {
            assert_eq!(
                validate_coff(&object).unwrap_err().code,
                "P1A_FORBIDDEN_BUILD_SIGNATURE"
            );
        }

        let temp = tempfile::tempdir().unwrap();
        for extension in PROVIDER_NATIVE_EXTENSIONS {
            let path = temp.path().join(format!("kernel.{extension}"));
            fs::write(&path, b"ordinary").unwrap();
            assert_eq!(
                scan_target(temp.path(), "test").unwrap_err().code,
                "P1A_FORBIDDEN_BUILD_ARTIFACT",
                "provider extension was accepted: {extension}"
            );
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn rejects_renamed_ptx_and_provider_elf_with_leading_trivia() {
        let temp = tempfile::tempdir().unwrap();
        let mut ptx = Vec::new();
        for part in [
            &b"\xef\xbb\xbf/* generated */ .ver"[..],
            &b"sion 8.8\n/* target */ .tar"[..],
            &b"get sm_120\n.address_size 64\n"[..],
        ] {
            ptx.extend_from_slice(part);
        }
        fs::write(temp.path().join("opaque.bin"), ptx).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );

        fs::remove_file(temp.path().join("opaque.bin")).unwrap();
        let mut cubin = vec![0u8; 64];
        cubin[..4].copy_from_slice(b"\x7fELF");
        cubin[5] = 1;
        cubin[18..20].copy_from_slice(&190u16.to_le_bytes());
        fs::write(temp.path().join("opaque.bin"), cubin).unwrap();
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_FORBIDDEN_BUILD_SIGNATURE"
        );
    }

    #[test]
    fn requires_materialized_clippy_and_test_targets_but_not_fmt() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            scan_target(temp.path(), "clippy").unwrap_err().code,
            "P1A_QUALITY_TARGET_EMPTY"
        );
        assert_eq!(
            scan_target(temp.path(), "test").unwrap_err().code,
            "P1A_QUALITY_TARGET_EMPTY"
        );
        assert_eq!(scan_target(temp.path(), "fmt").unwrap().file_count, 0);
        assert_eq!(
            scan_target(&temp.path().join("missing"), "test")
                .unwrap_err()
                .code,
            "P1A_QUALITY_TARGET_EMPTY"
        );
    }
}
