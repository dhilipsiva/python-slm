use crate::error::{IoContext, Result, XtaskError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const PROFILE: &str = "prototype-windows-5090-v1";
pub(crate) const ALLOCATION_BYTES: u64 = 2_952_790_016;

#[derive(Clone, Debug, Default)]
pub(crate) struct ProbeOptions {
    pub cuda_root: Option<PathBuf>,
    pub vs_instance_id: Option<String>,
    pub device_uuid: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProbeReport {
    value: Value,
    #[cfg(feature = "p2-cuda")]
    cuda_root: PathBuf,
}

impl ProbeReport {
    pub(crate) fn into_json(self) -> Value {
        self.value
    }

    #[cfg(feature = "p2-cuda")]
    pub(crate) fn canonical_sha256(&self) -> Result<String> {
        let bytes = serde_json::to_vec(&self.value).map_err(|error| {
            XtaskError::integrity(
                "P2_P1B_RESULT_INVALID",
                format!("could not canonicalize the typed P1B result: {error}"),
            )
        })?;
        Ok(crate::hash::bytes(&bytes))
    }

    #[cfg(feature = "p2-cuda")]
    pub(crate) fn cuda_root(&self) -> &Path {
        &self.cuda_root
    }

    #[cfg(feature = "p2-cuda")]
    pub(crate) fn field_str(&self, pointer: &'static str) -> Result<&str> {
        self.value
            .pointer(pointer)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P2_P1B_RESULT_INVALID",
                    format!("P1B result is missing string field {pointer}"),
                )
            })
    }

    #[cfg(feature = "p2-cuda")]
    pub(crate) fn field_u64(&self, pointer: &'static str) -> Result<u64> {
        self.value
            .pointer(pointer)
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P2_P1B_RESULT_INVALID",
                    format!("P1B result is missing integer field {pointer}"),
                )
            })
    }

    #[cfg(feature = "p2-cuda")]
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u32, u32, u32);
#[derive(Clone, Debug, Eq, PartialEq)]
struct Device {
    uuid: String,
    model: String,
    cc: String,
    vram: u64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Runtime {
    schema: String,
    status: String,
    device_uuid: String,
    device_model: String,
    compute_capability: String,
    total_vram_bytes: u64,
    allocation_bytes: u64,
    free_memory_before_bytes: u64,
    free_memory_during_bytes: u64,
    free_memory_after_bytes: u64,
    sentinel_first: u32,
    sentinel_last: u32,
    runtime_version: u32,
    driver_version: u32,
    cublas_version: u32,
    cublaslt_version: u64,
    synchronized: bool,
    owned_resources_released: bool,
}
pub(crate) fn probe(options: ProbeOptions) -> Result<Value> {
    probe_report(options).map(ProbeReport::into_json)
}

pub(crate) fn probe_report(options: ProbeOptions) -> Result<ProbeReport> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        windows::probe(options)
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = options;
        Err(XtaskError::gate(
            "DEFERRED_POST_P16",
            "probe-cuda is implemented only for native Windows x86_64",
            "Use the prototype Windows host; portable providers are deferred until P17.",
        ))
    }
}
fn parse_version(text: &str) -> Result<Version> {
    let p = text
        .strip_prefix('v')
        .unwrap_or(text)
        .split('.')
        .collect::<Vec<_>>();
    if !(2..=3).contains(&p.len())
        || p.iter()
            .any(|v| v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(XtaskError::integrity(
            "P1B_CUDA_VERSION_INVALID",
            "CUDA version is not dotted numeric text",
        ));
    }
    let n = |v: &str| {
        v.parse::<u32>().map_err(|_| {
            XtaskError::integrity("P1B_CUDA_VERSION_INVALID", "CUDA version overflowed")
        })
    };
    Ok(Version(
        n(p[0])?,
        n(p[1])?,
        if p.len() == 3 { n(p[2])? } else { 0 },
    ))
}
fn parse_targets(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        XtaskError::integrity("P1B_CUDA_TARGET_OUTPUT_INVALID", "nvcc output is not UTF-8")
    })?;
    Ok(text
        .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']'))
        .filter(|v| v.starts_with("sm_") || v.starts_with("compute_"))
        .map(str::to_owned)
        .collect())
}
fn select_version(values: &[(Version, BTreeSet<String>)]) -> Result<Version> {
    values
        .iter()
        .filter(|(v, t)| {
            *v >= Version(12, 8, 0) && t.contains("sm_120") && t.contains("compute_120")
        })
        .map(|(v, _)| *v)
        .max()
        .ok_or_else(|| {
            XtaskError::gate(
                "P1B_COMPATIBLE_CUDA_TOOLKIT_NOT_FOUND",
                "no CUDA toolkit at least 12.8 advertises both sm_120 and compute_120",
                "Install a compatible toolkit or pass --cuda-root.",
            )
        })
}
fn normalize_uuid(text: &str) -> Option<String> {
    let v = text
        .trim()
        .strip_prefix("GPU-")
        .unwrap_or(text.trim())
        .replace('-', "");
    if v.len() != 32 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = v.to_ascii_lowercase();
    Some(format!(
        "GPU-{}-{}-{}-{}-{}",
        &v[..8],
        &v[8..12],
        &v[12..16],
        &v[16..20],
        &v[20..]
    ))
}
fn select_device(devices: &[Device], wanted: Option<&str>) -> Result<Device> {
    let matched = devices
        .iter()
        .filter(|d| d.model == "NVIDIA GeForce RTX 5090" && d.cc == "12.0")
        .collect::<Vec<_>>();
    if let Some(wanted) = wanted {
        let wanted = normalize_uuid(wanted).ok_or_else(|| {
            XtaskError::new(
                "P1B_DEVICE_UUID_INVALID",
                crate::error::Category::Usage,
                "device UUID is malformed",
                "Use canonical GPU UUID text.",
            )
        })?;
        return matched
            .into_iter()
            .find(|d| d.uuid.eq_ignore_ascii_case(&wanted))
            .cloned()
            .ok_or_else(|| {
                XtaskError::gate(
                    "P1B_DEVICE_UUID_NOT_FOUND",
                    "the UUID is not a visible RTX 5090 with CC 12.0",
                    "Use an exact visible UUID.",
                )
            });
    }
    match matched.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(XtaskError::gate(
            "P1B_RTX5090_NOT_FOUND",
            "no visible RTX 5090 with CC 12.0 was found",
            "Expose the intended RTX 5090.",
        )),
        _ => Err(XtaskError::gate(
            "P1B_RTX5090_AMBIGUOUS",
            "multiple RTX 5090 devices match",
            "Pass --device-uuid.",
        )),
    }
}
fn native_failure_code(stderr: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(stderr).ok()?;
    text.lines().map(str::trim).find(|line| {
        line.starts_with("P1B_")
            && line.len() <= 96
            && line
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

fn validate_runtime(v: &Runtime) -> Result<()> {
    if v.schema != "python-slm-p1b-native-runtime-result-v1"
        || v.status != "PASS"
        || v.device_model != "NVIDIA GeForce RTX 5090"
        || v.compute_capability != "12.0"
        || normalize_uuid(&v.device_uuid).as_deref() != Some(v.device_uuid.as_str())
        || v.allocation_bytes != ALLOCATION_BYTES
        || v.sentinel_first != 42
        || v.sentinel_last != 42
        || !v.synchronized
        || !v.owned_resources_released
        || v.runtime_version == 0
        || v.driver_version == 0
        || v.cublas_version == 0
        || v.cublaslt_version == 0
    {
        return Err(XtaskError::integrity(
            "P1B_RUNTIME_RESULT_INVALID",
            "native CUDA result violated the fixed runtime contract",
        ));
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod windows {
    use super::*;
    use crate::p1a_process::{
        AuditedOutput, DirectCommand, ProcessPolicy, QualifiedPersistentFile,
    };
    use crate::p1a_windows::{
        FileIdentity, ToolFileIdentity, VisualStudioToolchain, WindowsSdkToolchain,
    };
    use regex::Regex;
    #[derive(Clone)]
    struct Toolkit {
        version: Version,
        root: PathBuf,
        bin: PathBuf,
        include: PathBuf,
        lib: PathBuf,
        nvcc: ToolFileIdentity,
        // Content identity, because CUDA 13.1 ships this one binary with no version
        // resource while every other tool here carries `6.14.11.9000`. Measured, not
        // assumed. Nothing in this module reads a version, so requiring one is a
        // requirement on how NVIDIA packaged the file rather than on its identity,
        // which the SHA-256 pins far more tightly than a version string would.
        cuobjdump: FileIdentity,
        compiler_tools: BTreeMap<&'static str, ToolFileIdentity>,
        windows_sdk: WindowsSdkToolchain,
        // Headers and import libraries, which are not PE images and so carry no
        // version resource to identify them by.
        files: BTreeMap<&'static str, FileIdentity>,
        targets: BTreeSet<String>,
    }
    struct Ctx<'a> {
        work: &'a Path,
        captures: &'a Path,
        n: usize,
    }

    struct OwnedTemp {
        path: Option<PathBuf>,
    }

    impl OwnedTemp {
        fn create() -> Result<Self> {
            let parent = fs::canonicalize(std::env::temp_dir()).io_context(
                "P1B_TEMP_CREATE_FAILED",
                "could not canonicalize the system temporary directory",
            )?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| {
                    XtaskError::environment(
                        "P1B_TEMP_CREATE_FAILED",
                        "the system clock is before the Unix epoch",
                    )
                })?
                .as_nanos();
            for attempt in 0_u32..128 {
                let path = parent.join(format!(
                    "python-slm-p1b-{:08x}-{now:032x}-{attempt:02x}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(Self { path: Some(path) }),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        return Err(XtaskError::environment(
                            "P1B_TEMP_CREATE_FAILED",
                            format!("could not create the private CUDA directory: {error}"),
                        ));
                    }
                }
            }
            Err(XtaskError::environment(
                "P1B_TEMP_CREATE_FAILED",
                "could not allocate a unique private CUDA directory",
            ))
        }

        fn path(&self) -> &Path {
            self.path.as_deref().expect("owned temp remains live")
        }

        fn close(mut self) -> Result<()> {
            let path = self.path.take().expect("owned temp remains live");
            fs::remove_dir_all(&path).io_context(
                "P1B_TEMP_REMOVE_FAILED",
                "could not remove the private CUDA directory",
            )
        }
    }

    impl Drop for OwnedTemp {
        fn drop(&mut self) {
            if let Some(path) = self.path.take() {
                let _ = fs::remove_dir_all(path);
            }
        }
    }

    pub(super) fn probe(o: ProbeOptions) -> Result<ProbeReport> {
        if o.cuda_root.as_deref().is_some_and(|p| !p.is_absolute()) {
            return Err(XtaskError::new(
                "P1B_CUDA_ROOT_NOT_ABSOLUTE",
                crate::error::Category::Usage,
                "--cuda-root must be absolute",
                "Pass one absolute toolkit root.",
            ));
        }
        if o.device_uuid
            .as_deref()
            .is_some_and(|v| normalize_uuid(v).is_none())
        {
            return Err(XtaskError::new(
                "P1B_DEVICE_UUID_INVALID",
                crate::error::Category::Usage,
                "device UUID is malformed",
                "Use canonical GPU UUID text.",
            ));
        }
        let temp = OwnedTemp::create()?;
        let work = temp.path().to_path_buf();
        let captures = work.join("captures");
        fs::create_dir(&captures)
            .io_context("P1B_CAPTURE_CREATE_FAILED", "could not create captures")?;
        let mut ctx = Ctx {
            work: &work,
            captures: &captures,
            n: 0,
        };
        let vs = visual_studio(&o, &mut ctx)?;
        let windows_sdk = crate::p1a_windows::discover_windows_sdk()?;
        let toolkit = toolkit(&o, &vs, &windows_sdk, &mut ctx)?;
        let source = work.join("probe.cu");
        fs::write(&source, include_bytes!("../probes/p1b_cuda_probe.cu"))
            .io_context("P1B_SOURCE_WRITE_FAILED", "could not materialize source")?;
        let mixed = work.join("mixed.exe");
        let ptx = work.join("ptx-only.exe");
        compile(&toolkit, &vs, &source, &mixed, false, &mut ctx)?;
        compile(&toolkit, &vs, &source, &ptx, true, &mut ctx)?;
        let mixed_images = inspect(&toolkit, &vs, &mixed, false, &mut ctx)?;
        let ptx_images = inspect(&toolkit, &vs, &ptx, true, &mut ctx)?;
        let mixed_run = execute(&toolkit, &vs, &mixed, o.device_uuid.as_deref(), &mut ctx)?;
        let device = select_device(
            &[Device {
                uuid: mixed_run.device_uuid.clone(),
                model: mixed_run.device_model.clone(),
                cc: mixed_run.compute_capability.clone(),
                vram: mixed_run.total_vram_bytes,
            }],
            Some(&mixed_run.device_uuid),
        )?;
        let ptx_run = execute(&toolkit, &vs, &ptx, Some(&device.uuid), &mut ctx)?;
        if mixed_run.device_uuid != ptx_run.device_uuid
            || mixed_run.driver_version != ptx_run.driver_version
            || mixed_run.runtime_version != ptx_run.runtime_version
            || mixed_run.cublas_version != ptx_run.cublas_version
            || mixed_run.cublaslt_version != ptx_run.cublaslt_version
        {
            return Err(XtaskError::integrity(
                "P1B_RUNTIME_IDENTITY_DRIFT",
                "the two artifacts observed different runtime identities",
            ));
        }
        let result = result(
            &toolkit,
            &vs,
            &device,
            &crate::p1a_windows::native_file_identity(&mixed)?,
            mixed_images,
            &mixed_run,
            &crate::p1a_windows::native_file_identity(&ptx)?,
            ptx_images,
            &ptx_run,
        )?;
        #[cfg(feature = "p2-cuda")]
        let cuda_root = toolkit.root.clone();
        temp.close()?;
        Ok(ProbeReport {
            value: result,
            #[cfg(feature = "p2-cuda")]
            cuda_root,
        })
    }
    fn visual_studio(o: &ProbeOptions, ctx: &mut Ctx<'_>) -> Result<VisualStudioToolchain> {
        let program = crate::p1a_windows::discover_vswhere_path()?;
        let lock = crate::p1a_windows::bind_vswhere_runtime()?;
        let f = lock.setup_configuration_identity();
        let out = run(
            ProcessPolicy::HostOnly,
            &program,
            crate::p1a_windows::VSWHERE_ARGS
                .iter()
                .map(OsString::from)
                .collect(),
            vec!["${VSWHERE}".to_owned()],
            ctx,
            Vec::new(),
            vec![QualifiedPersistentFile {
                path: f.path.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            }],
            BTreeMap::new(),
            30,
            "P1B_VSWHERE_FAILED",
            &["vswhere.exe"],
        )?;
        crate::p1a_windows::select_visual_studio(o.vs_instance_id.as_deref(), &out.stdout)
    }
    fn toolkit(
        o: &ProbeOptions,
        vs: &VisualStudioToolchain,
        windows_sdk: &WindowsSdkToolchain,
        ctx: &mut Ctx<'_>,
    ) -> Result<Toolkit> {
        let roots = if let Some(root) = o.cuda_root.as_deref() {
            vec![directory(root)?]
        } else {
            let (a, b) = crate::p1a_windows::native_program_files_roots()?;
            [a, b]
                .into_iter()
                .map(|p| p.join("NVIDIA GPU Computing Toolkit").join("CUDA"))
                .filter(|p| p.is_dir())
                .flat_map(|p| {
                    fs::read_dir(p)
                        .into_iter()
                        .flatten()
                        .filter_map(std::result::Result::ok)
                        .map(|e| e.path())
                })
                .collect()
        };
        let mut found = Vec::new();
        for root in roots {
            if !root.is_dir() {
                continue;
            }
            let root = directory(&root)?;
            let version =
                parse_version(root.file_name().and_then(OsStr::to_str).ok_or_else(|| {
                    XtaskError::integrity("P1B_CUDA_VERSION_INVALID", "CUDA directory is not UTF-8")
                })?)?;
            if version < Version(12, 8, 0) && o.cuda_root.is_none() {
                continue;
            }
            let mut t = bind_toolkit(root, version, windows_sdk.clone())?;
            let a = query_targets(&t, vs, "--list-gpu-arch", ctx)?;
            let b = query_targets(&t, vs, "--list-gpu-code", ctx)?;
            t.targets = a.union(&b).cloned().collect();
            found.push(t);
        }
        let selected = select_version(
            &found
                .iter()
                .map(|t| (t.version, t.targets.clone()))
                .collect::<Vec<_>>(),
        )?;
        found
            .into_iter()
            .filter(|t| t.version == selected)
            .max_by(|a, b| a.root.cmp(&b.root))
            .ok_or_else(|| {
                XtaskError::integrity("P1B_CUDA_SELECTION_FAILED", "selected toolkit vanished")
            })
    }
    fn bind_toolkit(
        root: PathBuf,
        version: Version,
        windows_sdk: WindowsSdkToolchain,
    ) -> Result<Toolkit> {
        let bin = directory(&root.join("bin"))?;
        let include = directory(&root.join("include"))?;
        let lib = directory(&root.join("lib").join("x64"))?;
        let id = |p: &Path| crate::p1a_windows::native_file_identity(p);
        // Content identity, not tool identity: a `.h` or a COFF `.lib` has no
        // version resource, and asking for one fails with a Win32 error that
        // names the file rather than the mistake.
        let content = |p: &Path| crate::p1a_windows::native_file_content_identity(p);
        let files = BTreeMap::from([
            ("cuda_header", content(&include.join("cuda.h"))?),
            (
                "cuda_runtime_header",
                content(&include.join("cuda_runtime.h"))?,
            ),
            ("cuda_import_library", content(&lib.join("cuda.lib"))?),
            ("cudart_import_library", content(&lib.join("cudart.lib"))?),
            ("cublas_import_library", content(&lib.join("cublas.lib"))?),
            (
                "cublaslt_import_library",
                content(&lib.join("cublasLt.lib"))?,
            ),
        ]);
        let compiler_tools = BTreeMap::from([
            ("ptxas", id(&bin.join("ptxas.exe"))?),
            ("nvlink", id(&bin.join("nvlink.exe"))?),
            ("fatbinary", id(&bin.join("fatbinary.exe"))?),
            ("cudafe++", id(&bin.join("cudafe++.exe"))?),
            ("cicc", id(&root.join("nvvm").join("bin").join("cicc.exe"))?),
        ]);
        Ok(Toolkit {
            version,
            nvcc: id(&bin.join("nvcc.exe"))?,
            cuobjdump: content(&bin.join("cuobjdump.exe"))?,
            compiler_tools,
            windows_sdk,
            root,
            bin,
            include,
            lib,
            files,
            targets: BTreeSet::new(),
        })
    }
    fn query_targets(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        arg: &str,
        ctx: &mut Ctx<'_>,
    ) -> Result<BTreeSet<String>> {
        let out = run(
            ProcessPolicy::CudaProbe,
            &t.nvcc.path,
            vec![arg.into()],
            vec!["${NVCC}".to_owned()],
            ctx,
            roots(t, vs),
            Vec::new(),
            environment(t, vs, ctx.work),
            30,
            "P1B_NVCC_TARGET_QUERY_FAILED",
            &["nvcc.exe"],
        )?;
        parse_targets(&out.stdout)
    }
    fn require_hash_bound_compiler_descendants(
        audit: &crate::p1a_process::ProcessAudit,
        expected: &[&ToolFileIdentity],
    ) -> Result<()> {
        let expected = expected
            .iter()
            .map(|identity| {
                let leaf = identity
                    .path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| {
                        XtaskError::integrity(
                            "P1B_TOOL_IDENTITY_INVALID",
                            "compiler identity has no UTF-8 leaf name",
                        )
                    })?
                    .to_ascii_lowercase();
                Ok((leaf, *identity))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        for process in audit.process_identities.iter().filter(|process| {
            process.process_id != audit.root_process_id
                || process.creation_time_100ns != audit.root_creation_time_100ns
        }) {
            let identity = expected
                .get(&process.executable_name.to_ascii_lowercase())
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "P1B_UNBOUND_TOOL_DESCENDANT",
                        "compiler descendant has no pre-bound identity",
                    )
                })?;
            if process.executable_sha256 != identity.sha256
                || process.executable_bytes != identity.bytes
            {
                return Err(XtaskError::integrity(
                    "P1B_TOOL_DESCENDANT_IDENTITY_MISMATCH",
                    "compiler descendant differs from its pre-bound identity",
                ));
            }
        }
        for module in &audit.loaded_modules {
            if let Some(identity) = expected.get(&module.module_name.to_ascii_lowercase())
                && (module.module_sha256 != identity.sha256
                    || module.module_bytes != identity.bytes)
            {
                return Err(XtaskError::integrity(
                    "P1B_TOOL_MODULE_IDENTITY_MISMATCH",
                    "compiler module differs from its pre-bound identity",
                ));
            }
        }
        Ok(())
    }

    fn compile(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        source: &Path,
        output: &Path,
        ptx: bool,
        ctx: &mut Ctx<'_>,
    ) -> Result<()> {
        let code = if ptx {
            "-gencode=arch=compute_120,code=compute_120"
        } else {
            "-gencode=arch=compute_120,code=[sm_120,compute_120]"
        };
        let args: Vec<OsString> = vec![
            "-m64".into(),
            "-std=c++17".into(),
            "-O2".into(),
            "--cudart=shared".into(),
            "-Xcompiler=/EHsc".into(),
            "-Xcompiler=/W4".into(),
            "-Xcompiler=/WX".into(),
            "-Xcompiler=/MD".into(),
            "-Xlinker=/WX".into(),
            code.into(),
            "-ccbin".into(),
            plain(&vs.cl.path),
            "-I".into(),
            plain(&t.include),
            "-L".into(),
            plain(&t.lib),
            plain(source),
            "-o".into(),
            plain(output),
            "cuda.lib".into(),
            "cublas.lib".into(),
            "cublasLt.lib".into(),
        ];
        // nvcc sends the host compiler's diagnostics through a shell redirect into
        // a temp file, so a rejection downstream of nvcc reaches us as a bare exit
        // 1. Carrying the exact invocation is what makes that reproducible by hand.
        let invocation = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let compiled = run(
            ProcessPolicy::CudaProbe,
            &t.nvcc.path,
            args,
            vec!["${NVCC}".to_owned(), code.to_owned()],
            ctx,
            roots(t, vs),
            Vec::new(),
            environment(t, vs, ctx.work),
            300,
            "P1B_CUDA_COMPILE_FAILED",
            &[
                "nvcc.exe",
                "cl.exe",
                "c1xx.dll",
                "c2.dll",
                "ptxas.exe",
                "nvlink.exe",
                "fatbinary.exe",
                "cudafe++.exe",
                "cicc.exe",
                "link.exe",
            ],
        )
        .map_err(|error| {
            XtaskError::new(
                error.code.clone(),
                crate::error::Category::Gate,
                format!(
                    "{}; nvcc {} ; host compiler {}",
                    error.message,
                    invocation,
                    vs.cl.path.display()
                ),
                error.remediation.clone(),
            )
        })?;
        require_hash_bound_compiler_descendants(
            &compiled.audit,
            &[
                &vs.cl,
                &vs.link,
                &vs.cpp_frontend,
                &vs.code_generator,
                &t.compiler_tools["ptxas"],
                &t.compiler_tools["nvlink"],
                &t.compiler_tools["fatbinary"],
                &t.compiler_tools["cudafe++"],
                &t.compiler_tools["cicc"],
            ],
        )?;
        let warning =
            Regex::new(r"(?im)(^|[^[:alpha:]])warning(?:\s+[A-Z0-9]+)?\s*[: ]").expect("regex");
        if warning.is_match(utf8(&compiled.stdout)?) || warning.is_match(utf8(&compiled.stderr)?) {
            return Err(XtaskError::integrity(
                "P1B_CUDA_COMPILE_WARNING",
                "CUDA compilation emitted a warning despite the warnings-as-errors policy",
            ));
        }
        if !output.is_file() {
            return Err(XtaskError::integrity(
                "P1B_CUDA_OUTPUT_MISSING",
                "nvcc created no executable",
            ));
        }
        Ok(())
    }
    fn validate_cuda_images(
        sass_targets: &BTreeSet<String>,
        ptx_targets: &BTreeSet<String>,
        sass: &[u8],
        ptx: &[u8],
        ptx_only: bool,
    ) -> Result<(bool, bool)> {
        let expected_sass = if ptx_only {
            BTreeSet::new()
        } else {
            BTreeSet::from(["sm_120".to_owned()])
        };
        if *sass_targets != expected_sass
            || *ptx_targets != BTreeSet::from(["compute_120".to_owned()])
        {
            return Err(XtaskError::integrity(
                "P1B_CUDA_IMAGE_SET_INVALID",
                "unexpected CUDA image set",
            ));
        }
        let sentinel = utf8(sass)?.contains("p1b_sentinel_kernel")
            || utf8(ptx)?.contains("p1b_sentinel_kernel");
        let encoded = Regex::new(r"(?i)\b[0-9a-f]{16}\b")
            .expect("regex")
            .is_match(utf8(sass)?);
        if !sentinel || (!ptx_only && !encoded) {
            return Err(XtaskError::integrity(
                "P1B_CUDA_SENTINEL_IMAGE_INVALID",
                "sentinel or encoded SASS is missing",
            ));
        }
        Ok((sentinel, encoded))
    }

    fn inspect(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        artifact: &Path,
        ptx_only: bool,
        ctx: &mut Ctx<'_>,
    ) -> Result<Value> {
        let cu = |arg: &str, ctx: &mut Ctx<'_>| {
            run(
                ProcessPolicy::CudaProbe,
                &t.cuobjdump.path,
                vec![arg.into(), artifact.as_os_str().to_owned()],
                vec!["${CUOBJDUMP}".to_owned()],
                ctx,
                roots(t, vs),
                Vec::new(),
                environment(t, vs, ctx.work),
                60,
                "P1B_CUOBJDUMP_FAILED",
                &["cuobjdump.exe"],
            )
        };
        let sass_targets = architecture(&cu("--list-elf", ctx)?.stdout, "sm_")?;
        let ptx_targets = architecture(&cu("--list-ptx", ctx)?.stdout, "compute_")?;
        let sass = cu("--dump-sass", ctx)?;
        let ptx = cu("--dump-ptx", ctx)?;
        let (sentinel, encoded) = validate_cuda_images(
            &sass_targets,
            &ptx_targets,
            &sass.stdout,
            &ptx.stdout,
            ptx_only,
        )?;
        let db = |arg: &str, ctx: &mut Ctx<'_>| {
            run(
                ProcessPolicy::CudaProbe,
                &vs.dumpbin.path,
                vec![arg.into(), artifact.as_os_str().to_owned()],
                vec!["${DUMPBIN}".to_owned()],
                ctx,
                roots(t, vs),
                Vec::new(),
                environment(t, vs, ctx.work),
                60,
                "P1B_DUMPBIN_FAILED",
                &["dumpbin.exe"],
            )
        };
        if !utf8(&db("/headers", ctx)?.stdout)?
            .to_ascii_lowercase()
            .contains("machine (x64)")
        {
            return Err(XtaskError::integrity(
                "P1B_PE_MACHINE_INVALID",
                "artifact is not x64 PE",
            ));
        }
        Ok(
            json!({"sass_targets": sass_targets, "ptx_targets": ptx_targets,
            "sentinel_kernel_present": sentinel, "encoded_sass_instruction_present": encoded,
            "pe_machine": "x86_64-pc-windows-msvc", "imports": imports(&db("/dependents", ctx)?.stdout)?}),
        )
    }
    fn execute(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        artifact: &Path,
        wanted: Option<&str>,
        ctx: &mut Ctx<'_>,
    ) -> Result<Runtime> {
        let mut args: Vec<OsString> = vec![
            "--allocation-bytes".into(),
            ALLOCATION_BYTES.to_string().into(),
        ];
        if let Some(wanted) = wanted {
            args.extend(["--device-uuid".into(), wanted.into()]);
        }
        let leaf = artifact
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| {
                XtaskError::integrity("P1B_ARTIFACT_NAME_INVALID", "artifact name is not UTF-8")
            })?;
        let out = run(
            ProcessPolicy::CudaProbe,
            artifact,
            args,
            vec!["${ARTIFACT}".to_owned()],
            ctx,
            roots(t, vs),
            Vec::new(),
            environment(t, vs, ctx.work),
            180,
            "P1B_RUNTIME_PROBE_FAILED",
            &[leaf],
        )?;
        let value = serde_json::from_slice(&out.stdout).map_err(|e| {
            XtaskError::integrity(
                "P1B_RUNTIME_JSON_INVALID",
                format!("runtime JSON invalid: {e}"),
            )
        })?;
        validate_runtime(&value)?;
        Ok(value)
    }
    #[allow(clippy::too_many_arguments)]
    fn run(
        policy: ProcessPolicy,
        program: &Path,
        args: Vec<OsString>,
        display_argv: Vec<String>,
        ctx: &mut Ctx<'_>,
        qualified_persistent_roots: Vec<PathBuf>,
        qualified_persistent_files: Vec<QualifiedPersistentFile>,
        environment: BTreeMap<String, Option<OsString>>,
        seconds: u64,
        code: &'static str,
        allowed: &[&str],
    ) -> Result<AuditedOutput> {
        ctx.n += 1;
        let display_argv = if display_argv.len() == args.len() + 1 {
            display_argv
        } else {
            display_argv
                .into_iter()
                .take(1)
                .chain((0..args.len()).map(|index| format!("arg-{index:02}")))
                .collect()
        };
        let out = crate::p1a_process::run(&DirectCommand {
            policy,
            program: program.to_path_buf(),
            args,
            display_argv,
            cwd: ctx.work.to_path_buf(),
            environment,
            timeout: Duration::from_secs(seconds),
            capture_directory: ctx.captures.to_path_buf(),
            capture_stem: format!("{:03}-command", ctx.n),
            qualified_persistent_roots,
            qualified_persistent_files,
        })?;
        let a = &out.audit;
        if out.exit_code != 0 {
            // Carry the tool's own words. Without them "exited 1" is the whole
            // report, and the difference between a missing compiler, a blocked
            // module load and a genuine device fault is invisible.
            let detail = String::from_utf8_lossy(&out.stderr);
            let detail = detail.trim();
            let detail = if detail.is_empty() {
                "no stderr".to_owned()
            } else {
                detail.chars().take(400).collect::<String>()
            };
            // The audit knows why more often than the tool does: a blocked module
            // load kills the process before it can say anything, and the exit code
            // alone cannot distinguish that from the tool rejecting its arguments.
            let blocked = if a.forbidden_modules.is_empty() && a.forbidden_processes.is_empty() {
                String::new()
            } else {
                format!(
                    " (forbidden modules {}; forbidden processes {})",
                    a.forbidden_modules.join(", "),
                    a.forbidden_processes.join(", ")
                )
            };
            return Err(XtaskError::new(
                native_failure_code(&out.stderr).unwrap_or(code),
                crate::error::Category::Gate,
                format!(
                    "contained CUDA command exited {}: {detail}{blocked}; loaded {}; stdout {} bytes {:?}",
                    out.exit_code,
                    a.executable_names.join(", "),
                    out.stdout.len(),
                    String::from_utf8_lossy(&out.stdout)
                        .chars()
                        .take(120)
                        .collect::<String>()
                ),
                "Correct CUDA/MSVC or the selected device.",
            ));
        }
        // Naming the failing conditions rather than reporting one boolean. Eleven
        // separate facts collapse into this check, and "incomplete" says which of
        // them held about as usefully as a corpus rejection count that does not say
        // which rule rejected. The MSVC toolchain in particular fails it by leaving
        // a telemetry process behind, which is a different problem from a device
        // fault and used to be indistinguishable from one here.
        let mut incomplete: Vec<String> = Vec::new();
        if !a.atomic_job_assignment {
            incomplete.push("the process was not assigned to its job atomically".to_owned());
        }
        if a.audited_process_count == 0 {
            incomplete.push("no process was audited".to_owned());
        }
        if a.covered_process_count != a.audited_process_count {
            incomplete.push(format!(
                "only {} of {} audited processes were covered",
                a.covered_process_count, a.audited_process_count
            ));
        }
        if a.successful_snapshots != a.audited_process_count {
            incomplete.push(format!(
                "only {} of {} audited processes were snapshotted",
                a.successful_snapshots, a.audited_process_count
            ));
        }
        if a.exit_races != 0 {
            incomplete.push(format!("{} processes exited mid-audit", a.exit_races));
        }
        if !a.forbidden_processes.is_empty() {
            incomplete.push(format!(
                "forbidden processes {}",
                a.forbidden_processes.join(", ")
            ));
        }
        if !a.forbidden_modules.is_empty() {
            incomplete.push(format!(
                "forbidden modules {}",
                a.forbidden_modules.join(", ")
            ));
        }
        if !a.process_tree_terminated {
            incomplete.push("the process tree did not terminate".to_owned());
        }
        if a.unexpected_descendants {
            incomplete.push(format!(
                "unexpected descendants among {}",
                a.executable_names.join(", ")
            ));
        }
        // `qualified_tool_descendants_cleaned` is deliberately *not* required. It
        // is only ever true when a qualified-tool survivor both appeared and was
        // cleaned, so demanding it inverts the gate: a run that leaves nothing
        // behind fails, and a run that leaks MSVC telemetry passes. Six of the
        // steps here declare no qualified persistent roots at all, and
        // `qualified_survivors` returns false immediately for those, so the
        // condition was unsatisfiable for them in every environment. The approved
        // P1A audit (`p1a.rs::process_audit_passed`) asserts
        // `!unexpected_descendants` and treats this flag as an observation, which
        // is the same safety property without the inversion — a survivor that is
        // not a qualified tool still fails, above.
        if a.timed_out {
            incomplete.push("the command timed out".to_owned());
        }
        if !incomplete.is_empty() {
            return Err(XtaskError::integrity(
                "P1B_PROCESS_AUDIT_FAILED",
                format!(
                    "CUDA process audit was incomplete: {}",
                    incomplete.join("; ")
                ),
            ));
        }
        let allowed = allowed
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let unexpected = a
            .executable_names
            .iter()
            .filter(|v| !allowed.contains(&v.to_ascii_lowercase()))
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected.is_empty() {
            return Err(XtaskError::integrity(
                "P1B_UNEXPECTED_TOOL_DESCENDANT",
                format!(
                    "unexpected compiler/runtime descendant {} (allowed {})",
                    unexpected.join(", "),
                    allowed.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
        Ok(out)
    }
    /// A path as a third-party tool can use it, without the verbatim prefix.
    ///
    /// Canonicalization produces `\\?\` paths on Windows, and that prefix turns
    /// off path normalization in the kernel: forward slashes stop being
    /// separators, `.` and `..` stop collapsing, and the 260-character rules stop
    /// applying. Tools that build paths by string concatenation break on it, and
    /// nvcc is one — it fails identically whether the prefix arrives through TEMP
    /// or through `-I`, `-L`, `-ccbin`, the source or the output, and reports
    /// nothing useful either way. Measured: the same compile exits 0 with plain
    /// paths and 1 with verbatim ones.
    ///
    /// The audit is unaffected. It keeps classifying and hashing the canonical
    /// path; this is only what gets handed to the tool.
    fn plain(path: &Path) -> OsString {
        path.to_str()
            .and_then(|text| text.strip_prefix(r"\\?\"))
            .map_or_else(|| path.as_os_str().to_owned(), OsString::from)
    }

    fn roots(t: &Toolkit, vs: &VisualStudioToolchain) -> Vec<PathBuf> {
        let mut roots = vec![
            t.root.clone(),
            vs.installation_path.clone(),
            t.windows_sdk.kits_root.clone(),
        ];
        // The VS Installer directory, which holds vswhere. It sits outside the
        // installation it describes — Microsoft puts it at a fixed location
        // precisely so tools can find VS without already knowing where VS is —
        // and the MSVC toolchain reaches for it during a compile. This audit
        // discovers and hashes that same binary itself as `vs.vswhere`, so the
        // directory is already trusted machinery here rather than a new one.
        if let Some(installer) = vs.vswhere.path.parent() {
            roots.push(installer.to_path_buf());
        }
        // The VS setup configuration store. cl.exe consults vswhere to resolve its
        // own installation, and vswhere loads
        // Microsoft.VisualStudio.Setup.Configuration.Native.dll from here — so the
        // observed tree during a compile is nvcc -> cmd -> cl -> vswhere, and this
        // is the last thing that chain reaches. Like the installer directory it is
        // Visual Studio's own machinery kept outside the installation it
        // describes.
        if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
            roots.push(
                PathBuf::from(program_data)
                    .join("Microsoft")
                    .join("VisualStudio")
                    .join("Setup"),
            );
        }
        roots
    }
    fn environment(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        work: &Path,
    ) -> BTreeMap<String, Option<OsString>> {
        let system =
            PathBuf::from(std::env::var_os("SYSTEMROOT").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32");
        let path = std::env::join_paths([
            t.bin.as_path(),
            vs.cl.path.parent().expect("cl parent"),
            system.as_path(),
        ])
        .expect("PATH");
        // TEMP and TMP go to third-party tools, so they must be ordinary paths.
        // The work root arrives canonicalized, and on Windows that means the
        // `\\?\` verbatim prefix, which switches off all path normalization in
        // the kernel: a tool that joins with a forward slash — nvcc does — then
        // produces a name with a literal `/` in it and fails to create the file.
        // nvcc reports that as `Could not open output file` on *stdout* and exits
        // 1, which is why this cost a while to find. The audited command still
        // runs with the canonical path; only the value handed to the tool is
        // plain.
        let plain_work = plain(work);
        // nvcc drives the host compiler through the CRT's system(), which finds
        // the shell by reading COMSPEC. The audit never inherits that variable,
        // deliberately, so it is set here to the same System32 image the forbidden
        // -image set already binds and hashes — naming the one interpreter the
        // owner permitted rather than leaving nvcc to search.
        let comspec = system.join("cmd.exe");
        // Observed during a compile: cl.exe reaches vswhere, reg.exe and
        // powershell.exe, none of which have anything to do with translating a
        // .cu file. That is Visual Studio's telemetry, and this is its documented
        // off switch. Turning the loader off is the honest fix — the alternative
        // is qualifying .NET Framework and a shell inside an audit whose purpose
        // is to refuse exactly those.
        let mut out = BTreeMap::from([
            ("PATH".to_owned(), Some(path)),
            ("COMSPEC".to_owned(), Some(comspec.into_os_string())),
            ("TEMP".to_owned(), Some(plain_work.clone())),
            ("TMP".to_owned(), Some(plain_work)),
            ("CUDA_PATH".to_owned(), Some(t.root.as_os_str().to_owned())),
            (
                "VSCMD_SKIP_SENDTELEMETRY".to_owned(),
                Some(OsString::from("1")),
            ),
        ]);
        for key in [
            "CUDA_HOME",
            "CUDNN_PATH",
            "HIP_PATH",
            "ROCM_PATH",
            "LIBTORCH",
            "PYTHONHOME",
            "PYTHONPATH",
            "CC",
            "CXX",
        ] {
            out.insert(key.to_owned(), None);
        }
        let sdk_include = t
            .windows_sdk
            .kits_root
            .join("Include")
            .join(&t.windows_sdk.version);
        let sdk_lib = t
            .windows_sdk
            .kits_root
            .join("Lib")
            .join(&t.windows_sdk.version);
        out.insert(
            "INCLUDE".to_owned(),
            Some(
                std::env::join_paths([
                    t.include.as_path(),
                    vs.msvc_include.as_path(),
                    sdk_include.join("ucrt").as_path(),
                    sdk_include.join("shared").as_path(),
                    sdk_include.join("um").as_path(),
                ])
                .expect("INCLUDE"),
            ),
        );
        out.insert(
            "LIB".to_owned(),
            Some(
                std::env::join_paths([
                    t.lib.as_path(),
                    vs.msvc_x64_lib.as_path(),
                    sdk_lib.join("ucrt").join("x64").as_path(),
                    sdk_lib.join("um").join("x64").as_path(),
                ])
                .expect("LIB"),
            ),
        );
        out
    }
    fn directory(path: &Path) -> Result<PathBuf> {
        let meta = fs::symlink_metadata(path)
            .io_context("P1B_CUDA_PATH_INVALID", "could not inspect CUDA path")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(XtaskError::integrity(
                "P1B_CUDA_PATH_INVALID",
                "CUDA directory is missing or linked",
            ));
        }
        fs::canonicalize(path)
            .io_context("P1B_CUDA_PATH_INVALID", "could not canonicalize CUDA path")
    }
    fn utf8(bytes: &[u8]) -> Result<&str> {
        std::str::from_utf8(bytes).map_err(|_| {
            XtaskError::integrity("P1B_TOOL_OUTPUT_INVALID", "tool output is not UTF-8")
        })
    }
    fn architecture(bytes: &[u8], prefix: &str) -> Result<BTreeSet<String>> {
        let regex = Regex::new(&format!(r"\b{}[0-9]+\b", regex::escape(prefix))).expect("regex");
        Ok(regex
            .find_iter(utf8(bytes)?)
            .map(|v| v.as_str().to_owned())
            .collect())
    }
    fn imports(bytes: &[u8]) -> Result<BTreeSet<String>> {
        let regex = Regex::new(r"(?i)\b[A-Za-z0-9_.+-]+\.dll\b").expect("regex");
        let values = regex
            .find_iter(utf8(bytes)?)
            .map(|v| v.as_str().to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let fixed = [
            "kernel32.dll",
            "advapi32.dll",
            "user32.dll",
            "ole32.dll",
            "oleaut32.dll",
            "shell32.dll",
            "ucrtbase.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "msvcp140.dll",
            "nvcuda.dll",
        ];
        if values.is_empty()
            || values.iter().any(|v| {
                !fixed.contains(&v.as_str())
                    && !v.starts_with("api-ms-win-")
                    && !v.starts_with("ext-ms-win-")
                    && !v.starts_with("cudart64_")
                    && !v.starts_with("cublas64_")
                    && !v.starts_with("cublaslt64_")
            })
        {
            return Err(XtaskError::integrity(
                "P1B_PE_IMPORT_UNEXPECTED",
                "unexpected or empty DLL import set",
            ));
        }
        Ok(values)
    }
    /// The three fields the probe result records for any pinned file.
    ///
    /// Executables are identified with their version resource as well, but the
    /// result never serializes it, so headers and import libraries — which have
    /// no version resource to query — carry exactly the same weight here.
    trait PinnedFile {
        fn path(&self) -> &Path;
        fn sha256(&self) -> &str;
        fn bytes(&self) -> u64;
    }
    impl PinnedFile for ToolFileIdentity {
        fn path(&self) -> &Path {
            &self.path
        }
        fn sha256(&self) -> &str {
            &self.sha256
        }
        fn bytes(&self) -> u64 {
            self.bytes
        }
    }
    impl PinnedFile for FileIdentity {
        fn path(&self) -> &Path {
            &self.path
        }
        fn sha256(&self) -> &str {
            &self.sha256
        }
        fn bytes(&self) -> u64 {
            self.bytes
        }
    }
    fn token(i: &impl PinnedFile, root: &Path, name: &str) -> Result<Value> {
        let rel = i.path().strip_prefix(root).map_err(|_| {
            XtaskError::integrity(
                "P1B_PATH_TOKENIZATION_FAILED",
                "identity escaped token root",
            )
        })?;
        Ok(
            json!({"path": format!("{name}/{}", rel.to_string_lossy().replace('\\', "/")), "sha256": i.sha256(), "bytes": i.bytes()}),
        )
    }
    fn runtime(v: &Runtime) -> Value {
        json!({"runtime_version": v.runtime_version, "driver_version": v.driver_version,
        "cublas_version": v.cublas_version, "cublaslt_version": v.cublaslt_version,
        "free_memory_before_bytes": v.free_memory_before_bytes, "free_memory_during_bytes": v.free_memory_during_bytes,
        "free_memory_after_bytes": v.free_memory_after_bytes, "allocation_bytes": v.allocation_bytes,
        "sentinel_first": v.sentinel_first, "sentinel_last": v.sentinel_last,
        "synchronized": v.synchronized, "owned_resources_released": v.owned_resources_released})
    }

    #[allow(clippy::too_many_arguments)]
    fn result(
        t: &Toolkit,
        vs: &VisualStudioToolchain,
        d: &Device,
        mixed_id: &ToolFileIdentity,
        mixed_images: Value,
        mixed: &Runtime,
        ptx_id: &ToolFileIdentity,
        ptx_images: Value,
        ptx: &Runtime,
    ) -> Result<Value> {
        let work = mixed_id.path.parent().ok_or_else(|| {
            XtaskError::integrity("P1B_WORK_ROOT_INVALID", "artifact has no parent")
        })?;
        let mut files = serde_json::Map::new();
        for (name, file) in &t.files {
            files.insert((*name).to_owned(), token(file, &t.root, "${CUDA_ROOT}")?);
        }
        let mut compiler_tools = serde_json::Map::new();
        for (name, file) in &t.compiler_tools {
            compiler_tools.insert((*name).to_owned(), token(file, &t.root, "${CUDA_ROOT}")?);
        }
        Ok(
            json!({"schema": "python-slm-p1b-cuda-probe-result-v1", "status": "PROBE_OK",
            "qualification_status": "SKIPPED", "profile": PROFILE,
            "toolkit": {"version": format!("{}.{}.{}", t.version.0, t.version.1, t.version.2), "root": "${CUDA_ROOT}",
                "nvcc": token(&t.nvcc, &t.root, "${CUDA_ROOT}")?, "cuobjdump": token(&t.cuobjdump, &t.root, "${CUDA_ROOT}")?,
                "compiler_tools": compiler_tools, "required_files": files, "supported_targets": t.targets},
            "compiler": {"visual_studio_instance_id": vs.selected_instance_id, "visual_studio_version": vs.installation_version,
                "msvc_tools_version": vs.msvc_tools_version, "windows_sdk_version": t.windows_sdk.version,
                "cl": token(&vs.cl, &vs.installation_path, "${VS_INSTALL}")?,
                "link": token(&vs.link, &vs.installation_path, "${VS_INSTALL}")?, "dumpbin": token(&vs.dumpbin, &vs.installation_path, "${VS_INSTALL}")?,
                "language_standard": "c++17", "warnings_as_errors": true, "runtime_linkage": "cuda-driver-plus-shared-runtime"},
            "driver_version": mixed.driver_version, "device": {"uuid": d.uuid, "model": d.model, "compute_capability": d.cc, "total_vram_bytes": d.vram},
            "runtime_libraries": {"cuda_driver": mixed.driver_version, "cuda_runtime": mixed.runtime_version, "cublas": mixed.cublas_version, "cublaslt": mixed.cublaslt_version},
            "mixed_artifact": {"kind": "sm_120-plus-compute_120", "executable": token(mixed_id, work, "${WORK}")?, "inspection": mixed_images, "execution": runtime(mixed)},
            "ptx_only_artifact": {"kind": "compute_120-ptx-only", "executable": token(ptx_id, work, "${WORK}")?, "inspection": ptx_images, "execution": runtime(ptx)},
            "allocation_bytes": ALLOCATION_BYTES, "sentinel_value": 42,
            "cleanup": {"temporary_directory_removed": true, "persistent_artifacts_written": false, "receipts_written": false}}),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cuda_image_fixtures_are_exact_and_fail_closed() {
            let sm = BTreeSet::from(["sm_120".to_owned()]);
            let ptx = BTreeSet::from(["compute_120".to_owned()]);
            validate_cuda_images(
                &sm,
                &ptx,
                b"p1b_sentinel_kernel /* 0123456789abcdef */",
                b"p1b_sentinel_kernel",
                false,
            )
            .unwrap();
            validate_cuda_images(
                &BTreeSet::new(),
                &ptx,
                b"",
                b".visible .entry p1b_sentinel_kernel",
                true,
            )
            .unwrap();
            assert_eq!(
                validate_cuda_images(&BTreeSet::new(), &ptx, b"", b"sentinel", false)
                    .unwrap_err()
                    .code,
                "P1B_CUDA_IMAGE_SET_INVALID"
            );
            assert_eq!(
                validate_cuda_images(&sm, &ptx, b"p1b_sentinel_kernel", b"", false)
                    .unwrap_err()
                    .code,
                "P1B_CUDA_SENTINEL_IMAGE_INVALID"
            );
            let extra = BTreeSet::from(["sm_120".to_owned(), "sm_121".to_owned()]);
            assert_eq!(
                validate_cuda_images(
                    &extra,
                    &ptx,
                    b"p1b_sentinel_kernel 0123456789abcdef",
                    b"",
                    false
                )
                .unwrap_err()
                .code,
                "P1B_CUDA_IMAGE_SET_INVALID"
            );
        }

        #[test]
        fn pe_import_fixture_accepts_only_windows_and_selected_cuda_closure() {
            let valid = b"KERNEL32.dll api-ms-win-core-file-l1-1-0.dll nvcuda.dll cudart64_131.dll cublas64_13.dll cublasLt64_13.dll";
            assert!(imports(valid).is_ok());
            assert_eq!(
                imports(b"kernel32.dll python313.dll").unwrap_err().code,
                "P1B_PE_IMPORT_UNEXPECTED"
            );
            assert_eq!(
                imports(b"no imports").unwrap_err().code,
                "P1B_PE_IMPORT_UNEXPECTED"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|v| (*v).to_owned()).collect()
    }
    #[test]
    fn toolkit_selection_accepts_128_129_and_13x() {
        let v = ["12.7", "12.8", "12.9", "13.1"]
            .into_iter()
            .map(|x| (parse_version(x).unwrap(), set(&["sm_120", "compute_120"])))
            .collect::<Vec<_>>();
        assert_eq!(select_version(&v).unwrap(), parse_version("13.1").unwrap());
        assert_eq!(
            select_version(&v[..1]).unwrap_err().code,
            "P1B_COMPATIBLE_CUDA_TOOLKIT_NOT_FOUND"
        );
        assert_eq!(
            select_version(&[(parse_version("13.0").unwrap(), set(&["sm_120"]))])
                .unwrap_err()
                .code,
            "P1B_COMPATIBLE_CUDA_TOOLKIT_NOT_FOUND"
        );
    }
    fn gpu(id: u8, model: &str, cc: &str) -> Device {
        Device {
            uuid: format!("GPU-00000000-0000-0000-0000-{id:012}"),
            model: model.to_owned(),
            cc: cc.to_owned(),
            vram: 34_000_000_000,
        }
    }
    #[test]
    fn device_selection_is_deterministic() {
        let d = [
            gpu(1, "NVIDIA GeForce RTX 5090", "12.0"),
            gpu(2, "NVIDIA GeForce RTX 5090", "12.0"),
            gpu(3, "NVIDIA GeForce RTX 4090", "8.9"),
        ];
        assert_eq!(
            select_device(&[], None).unwrap_err().code,
            "P1B_RTX5090_NOT_FOUND"
        );
        assert_eq!(
            select_device(&d, None).unwrap_err().code,
            "P1B_RTX5090_AMBIGUOUS"
        );
        assert_eq!(select_device(&d, Some(&d[1].uuid)).unwrap(), d[1]);
        assert_eq!(
            select_device(&d, Some("GPU-ffffffff-ffff-ffff-ffff-ffffffffffff"))
                .unwrap_err()
                .code,
            "P1B_DEVICE_UUID_NOT_FOUND"
        );
        assert_eq!(
            select_device(&d[1..], None).unwrap(),
            d[1],
            "other GPU models must not affect sole-5090 selection"
        );
    }

    #[test]
    fn native_failure_codes_are_preserved_only_when_closed() {
        assert_eq!(
            native_failure_code(b"P1B_ALLOCATION_FAILED\n"),
            Some("P1B_ALLOCATION_FAILED")
        );
        assert_eq!(
            native_failure_code(b"P1B_ALLOCATION_FAILED: C:\\secret\n"),
            None
        );
        assert_eq!(native_failure_code(&[0xff]), None);
    }

    #[test]
    fn target_and_runtime_parsers_fail_closed() {
        assert_eq!(
            parse_targets(
                b"sm_120
compute_120
sm_120"
            )
            .unwrap(),
            set(&["sm_120", "compute_120"])
        );
        assert_eq!(
            parse_targets(&[0xff]).unwrap_err().code,
            "P1B_CUDA_TARGET_OUTPUT_INVALID"
        );
        let mut runtime = Runtime {
            schema: "python-slm-p1b-native-runtime-result-v1".to_owned(),
            status: "PASS".to_owned(),
            device_uuid: "GPU-00000000-0000-0000-0000-000000000001".to_owned(),
            device_model: "NVIDIA GeForce RTX 5090".to_owned(),
            compute_capability: "12.0".to_owned(),
            total_vram_bytes: 34_000_000_000,
            allocation_bytes: ALLOCATION_BYTES,
            free_memory_before_bytes: 30,
            free_memory_during_bytes: 27,
            free_memory_after_bytes: 30,
            sentinel_first: 42,
            sentinel_last: 42,
            runtime_version: 13_010,
            driver_version: 13_010,
            cublas_version: 130_100,
            cublaslt_version: 130_100,
            synchronized: true,
            owned_resources_released: true,
        };
        validate_runtime(&runtime).unwrap();
        runtime.owned_resources_released = false;
        assert_eq!(
            validate_runtime(&runtime).unwrap_err().code,
            "P1B_RUNTIME_RESULT_INVALID"
        );
    }
}
