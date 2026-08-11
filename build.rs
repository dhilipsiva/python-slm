use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    for key in [
        "CUDA_PATH",
        "CUDNN_PATH",
        "CUDA_COMPUTE_CAP",
        "RUST_LLM_ALLOW_CUDA_13",
        "VCToolsInstallDir",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    for file in ["kernels/msvc_probe.cpp", "kernels/cuda_probe.cu"] {
        println!("cargo:rerun-if-changed={file}");
    }

    if env::var_os("CARGO_FEATURE_CUDA_MSVC_LINK").is_none() {
        return;
    }

    let target = env::var("TARGET").expect("Cargo did not set TARGET");
    assert!(
        target == "x86_64-pc-windows-msvc",
        "cuda-msvc-link requires target x86_64-pc-windows-msvc, got {target}"
    );

    let cuda = required_dir("CUDA_PATH");
    let cudnn = env::var_os("CUDNN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| cuda.clone());
    let nvcc = cuda.join("bin").join("nvcc.exe");
    let cuda_include = cuda.join("include");
    let cudnn_include = first_containing(
        &[
            cudnn.join("include").join("13.1"),
            cudnn.join("include").join("12.9"),
            cudnn.join("include").join("12.8"),
            cudnn.join("include"),
        ],
        "cudnn.h",
    )
    .unwrap_or_else(|| cudnn.join("include"));
    let cuda_lib = cuda.join("lib").join("x64");
    let cudnn_lib = first_containing(
        &[
            cudnn.join("lib").join("13.1").join("x64"),
            cudnn.join("lib").join("12.9").join("x64"),
            cudnn.join("lib").join("12.8").join("x64"),
            cudnn.join("lib").join("x64"),
        ],
        "cudnn.lib",
    )
    .unwrap_or_else(|| cudnn.join("lib").join("x64"));

    require_file(&nvcc, "nvcc.exe");
    for (dir, library) in [
        (&cuda_lib, "cudart.lib"),
        (&cuda_lib, "cublas.lib"),
        (&cuda_lib, "curand.lib"),
        (&cudnn_lib, "cudnn.lib"),
    ] {
        require_file(&dir.join(library), library);
    }
    require_file(&cudnn_include.join("cudnn.h"), "cudnn.h");

    validate_cuda_version(&nvcc);

    let cap = env::var("CUDA_COMPUTE_CAP").unwrap_or_else(|_| "120".to_owned());
    assert!(
        cap == "120",
        "RTX 5090 is SM120; set CUDA_COMPUTE_CAP=120 (got {cap})"
    );

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo did not set OUT_DIR"));
    let msvc_obj = out.join("msvc_probe.obj");
    let cuda_obj = out.join("cuda_probe.obj");
    let archive = out.join("rust_llm_cuda_probe.lib");

    run(
        Command::new("cl.exe")
            .args(["/nologo", "/std:c++17", "/O2", "/MD", "/c"])
            .arg("kernels/msvc_probe.cpp")
            .arg(format!("/Fo{}", msvc_obj.display())),
        "cl.exe (run from an x64 VS 2022 Developer PowerShell)",
    );

    run(
        Command::new(&nvcc)
            .args(["-c", "kernels/cuda_probe.cu", "-std=c++17", "-O3"])
            .arg(format!("-arch=sm_{cap}"))
            .arg("-ccbin=cl.exe")
            .arg("-Xcompiler=/MD")
            .arg("-I")
            .arg(&cuda_include)
            .arg("-I")
            .arg(&cudnn_include)
            .arg("-o")
            .arg(&cuda_obj),
        "nvcc.exe",
    );

    run(
        Command::new("lib.exe")
            .arg("/nologo")
            .arg(format!("/OUT:{}", archive.display()))
            .arg(&msvc_obj)
            .arg(&cuda_obj),
        "lib.exe (run from an x64 VS 2022 Developer PowerShell)",
    );

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", cuda_lib.display());
    println!("cargo:rustc-link-search=native={}", cudnn_lib.display());
    println!("cargo:rustc-link-lib=static=rust_llm_cuda_probe");
    for library in ["cudart", "cublas", "curand", "cudnn"] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}

fn required_dir(key: &str) -> PathBuf {
    let value = env::var_os(key).unwrap_or_else(|| {
        panic!("{key} is required by the cuda-msvc-link feature (use CUDA 12.8 or 12.9)")
    });
    let path = PathBuf::from(value);
    assert!(
        path.is_dir(),
        "{key} is not a directory: {}",
        path.display()
    );
    path
}

fn first_containing(paths: &[PathBuf], filename: &str) -> Option<PathBuf> {
    paths
        .iter()
        .find(|path| path.join(filename).is_file())
        .cloned()
}

fn require_file(path: &Path, label: &str) {
    assert!(
        path.is_file(),
        "required {label} was not found at {}",
        path.display()
    );
}

fn run(command: &mut Command, label: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to start {label}: {error}"));
    assert!(status.success(), "{label} failed with {status}");
}

fn validate_cuda_version(nvcc: &Path) {
    let output = Command::new(nvcc)
        .arg("--version")
        .output()
        .expect("failed to run nvcc --version");
    assert!(output.status.success(), "nvcc --version failed");
    let text = String::from_utf8_lossy(&output.stdout);
    let release = text
        .split("release ")
        .nth(1)
        .and_then(|tail| tail.split(',').next())
        .unwrap_or_else(|| panic!("could not parse CUDA version from nvcc output: {text}"));
    let mut parts = release.trim().split('.');
    let major: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let allow_13 = env::var_os("RUST_LLM_ALLOW_CUDA_13").is_some_and(|value| value == "1");
    assert!(
        (major == 12 && minor >= 8) || (allow_13 && major == 13),
        "RTX 5090 requires CUDA >=12.8; this reproducible target expects CUDA 12.8/12.9, got {release}. Set RUST_LLM_ALLOW_CUDA_13=1 only for an intentional CUDA 13 validation run."
    );
}
