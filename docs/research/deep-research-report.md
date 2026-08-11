# Pure-Rust LLM Pre-Training Pipeline for Windows MSVC and RTX 5090

## Feasibility, corrections, and recommended design

A zero-Python training stack on Windows 11 is technically viable with Rust as the orchestration, data-processing, model, and training language, while CUDA/C++ remains an optional low-level kernel boundary. The strongest current Rust-native route is **Burn 0.21 + burn-cuda/CubeCL**, with a small custom CUDA/CUTLASS FFI feature retained as an escape hatch for attention kernels that do not reach the required throughput. Burn's CUDA backend is implemented using CubeCL and `cudarc`; CubeCL is specifically designed to generate GPU kernels from Rust and supports CUDA backends. Burn 0.20 added CubeCL Flash Attention and Blackwell-oriented MMA/TMA work, while Burn 0.21 added further attention/autotuning and framework-overhead improvements. citeturn25search0turn25search13turn20search0

The target GPU is appropriate for the experiment. NVIDIA specifies the RTX 5090 as a Blackwell GPU with **21,760 CUDA cores, 32 GB GDDR7, a 512-bit interface, and 1,792 GB/s memory bandwidth**. citeturn21search2turn21search5 NVIDIA's Blackwell architecture documentation gives the RTX 5090 approximately **209.5 TFLOP/s dense BF16 Tensor-Core throughput with FP32 accumulation**, with substantially higher theoretical FP8 throughput. citeturn10view0

There are, however, four important corrections to the proposed specification.

| Item | Proposed specification | Research conclusion |
|---|---:|---|
| Model size | “~135M”, `d_model=768`, `d_ff=2048`, 12 layers | **124.67M parameters** with an untied LM head; only **100.09M** if embeddings are tied |
| Exactly ~135M | Same dimensions | Change only `d_ff` to **2432** → **135.29M** with untied LM head |
| Training speed | `>70,000 tok/s` | 70k finishes 2B tokens in **7h 56m 34s**, leaving only **3m 49s** of an eight-hour budget |
| FlashAttention-3 | Expose FA3 to RTX 5090 | Upstream FA3 targets **Hopper H100/H800**, not consumer Blackwell; use CubeCL Flash Attention or a Blackwell-native CUDA/CUTLASS kernel instead. citeturn0search3 |

The exact throughput arithmetic is:

\[
\frac{2,000,000,000}{28,800}
=69,444.44\ {\rm tokens/s}.
\]

Thus, **69.44k tok/s is merely the mathematical break-even point**, with no allowance for startup, JIT compilation, autotuning, checkpoints, dataloader stalls, synchronization, or OS overhead. At 70k tok/s the raw training work consumes 28,571 seconds. A practical acceptance target should instead be roughly **80–85k tok/s**:

| Sustained rate | Raw time for 2B tokens | Eight-hour margin |
|---:|---:|---:|
| 70k tok/s | 7h 56m | 3.8 min |
| 75k tok/s | 7h 24m | 35.6 min |
| 80k tok/s | 6h 57m | 63.3 min |
| 85k tok/s | 6h 32m | 87.8 min |

The compute budget does not make 80k obviously impossible. For the requested 2048-context, `d_ff=2048` architecture, a matrix-operation estimate including Q/K/V/O, SwiGLU, full 2048-token attention, and the 32k output projection is about **0.83 GFLOP per training token** under the usual forward-plus-backward approximation. At 70k tok/s that is about 58 TFLOP/s, and at 80k about 66 TFLOP/s. For the corrected 135.29M model with `d_ff=2432`, the corresponding estimate is about 0.89 GFLOP/token and 71 TFLOP/s at 80k tok/s. Those figures are well below the RTX 5090's theoretical BF16 Tensor-Core peak, but kernel efficiency, launch overhead, attention efficiency, and tensor shapes determine whether that theoretical headroom becomes real throughput. citeturn10view0

**I would therefore make BF16 + native GQA Flash Attention the production target. FP8 should be a second-stage optimization, not a dependency for meeting the initial SLA.** Blackwell hardware supports FP8 acceleration, and CubeCL has Blackwell-specific MMA capabilities, but end-to-end FP8 training also requires scaling/amax policy, suitable accumulation precision, stable optimizer state handling, and kernel coverage. BF16 is a much less risky baseline. citeturn10view0turn13search1

The eight-hour constraint should also apply to the **training run after corpus materialization**, not to downloading, AST-parsing, deduplicating, BPE-training, tokenizing, and then training. This distinction matters especially for The Stack v2: as of August 11, 2026, the official dataset is enormous—67.5 TB full and 32.1 TB deduplicated—and its Hugging Face dataset currently contains **SWHIDs rather than the actual file contents**, so source objects must additionally be retrieved. The dataset was updated to v2.2.0 for removals through July 29, 2026. citeturn21search0turn21search6 A total eight-hour network-to-trained-model SLA would therefore not be defensible; an eight-hour **GPU pretraining** SLA is.

## Rust data ingestion, filtering, deduplication, and token storage

### Source architecture

The data subsystem should have four bounded stages rather than allowing Tokio tasks, Rayon jobs, and tokenizer workers to compete without back-pressure:

```text
           asynchronous / I/O                    parallel / CPU
┌───────────────────────────────┐        ┌──────────────────────────────┐
│ reqwest / Parquet manifest    │        │ tree-sitter Python          │
│ downloads                     │───────►│ syntax + quality filtering  │
└───────────────────────────────┘  mpsc  └──────────────┬───────────────┘
                                                       │
                                                       ▼
                                            ┌──────────────────────┐
                                            │ MinHash + LSH dedup  │
                                            └──────────┬───────────┘
                                                       │
                                                       ▼
                                            ┌──────────────────────┐
                                            │ Tokenizers BPE       │
                                            │ + u16 shard writer   │
                                            └──────────────────────┘
```

Arrow's current native Rust Parquet crate has synchronous `RecordBatch` readers as well as `ParquetRecordBatchStreamBuilder`/`AsyncFileReader` for asynchronous sources, so there is no need for a Python/pyarrow process. citeturn18search6turn22search1turn22search4turn22search10 Rayon is a good match for AST parsing and MinHash because its parallel iterators distribute CPU work across a dedicated work-stealing pool. citeturn22search2turn22search5

There is one source-specific correction: for datasets that really contain a `content` column in Parquet, Arrow can emit source strings directly. **The Stack v2 itself is not such a dataset today**; its Hugging Face records contain IDs/provenance from which content is retrieved. Consequently, define the source behind a trait:

```rust
pub struct RawDocument {
    pub source_id: String,
    pub repo_id: Option<String>,
    pub license: Option<String>,
    pub content: Vec<u8>,
}

#[async_trait::async_trait]
pub trait DocumentSource: Send {
    async fn next_document(&mut self) -> anyhow::Result<Option<RawDocument>>;
}
```

Then implement both `ParquetContentSource` and `StackV2Source`. The latter decodes Stack metadata with Arrow and obtains the corresponding content through the source mechanism documented by BigCode. This also lets provenance and license information survive the curation process, which is important because The Stack v2 explicitly contains source under many different original licenses and requests compliance with those license terms. citeturn21search0

### AST and quality filter

Tree-sitter-python 0.25 exposes its Python `LANGUAGE` object to the Rust Tree-sitter parser; the official example uses `tree.root_node().has_error()` to determine whether the resulting tree contains syntax errors. citeturn19search0 That gives a precise implementation of the requested hard syntax gate without shelling out to CPython.

A practical `curation.rs` core is:

```rust
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::sync::Arc;
use tree_sitter::{Node, Parser, Tree};

const MIN_BYTES: usize = 100;

const GENERATED_MARKERS: &[&[u8]] = &[
    b"auto-generated",
    b"autogenerated",
    b"automatically generated",
    b"generated by",
    b"do not edit",
    b"do not modify",
    b"@generated",
];

#[derive(Debug, Clone)]
pub struct AcceptedDocument {
    pub source_id: String,
    pub repo_id: Option<String>,
    pub license: Option<String>,
    pub content: Arc<[u8]>,
}

fn make_python_parser() -> Result<Parser> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser
        .set_language(&language.into())
        .context("loading tree-sitter-python")?;
    Ok(parser)
}

fn has_generated_marker(src: &[u8]) -> bool {
    // Flags are ASCII in practice. Do the conversion once per candidate.
    let lower: Vec<u8> = src.iter().map(u8::to_ascii_lowercase).collect();

    GENERATED_MARKERS
        .iter()
        .any(|needle| lower.windows(needle.len()).any(|w| w == *needle))
}

fn comment_bytes(node: Node<'_>) -> usize {
    let mut total = 0usize;

    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if node.kind() == "comment" {
            total += node.end_byte().saturating_sub(node.start_byte());
            continue;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    total
}

fn passes_filter(parser: &mut Parser, src: &[u8]) -> bool {
    if src.len() < MIN_BYTES || has_generated_marker(src) {
        return false;
    }

    let Some(tree): Option<Tree> = parser.parse(src, None) else {
        return false;
    };

    let root = tree.root_node();

    // Hard reject syntax errors.
    if root.has_error() {
        return false;
    }

    let comments = comment_bytes(root);

    // User-specified rule: comment bytes may not exceed 50% of file bytes.
    comments.saturating_mul(2) <= src.len()
}

pub fn filter_batch(
    docs: Vec<AcceptedDocument>,
) -> Result<Vec<AcceptedDocument>> {
    docs.into_par_iter()
        .map_init(
            || make_python_parser().expect("tree-sitter init"),
            |parser, doc| {
                if passes_filter(parser, &doc.content) {
                    Some(doc)
                } else {
                    None
                }
            },
        )
        .while_some()
        .collect::<Vec<_>>()
        .pipe(Ok)
}

// Small convenience trait to avoid a temporary.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}
```

For production, I would avoid allocating a lower-cased copy just to detect boilerplate; use an ASCII case-insensitive SIMD/memchr matcher. The important architectural detail is that **each Rayon worker owns its Tree-sitter parser** instead of locking a shared parser.

Docstrings should not automatically count as comments. Tree-sitter identifies actual `#...` comments as comment nodes while Python docstrings remain expression/string syntax; preserving them is useful because docstrings contain function semantics and natural-language/code correspondences.

### Parallel near-deduplication

A corpus-wide all-pairs similarity test is the wrong implementation: it scales quadratically. Use MinHash to produce compact signatures in Rayon and then an LSH candidate index.

A useful configuration for Python is:

```text
normalization     normalize CRLF → LF; otherwise preserve code
shingle           5 logical lines, or 5 lexical tokens
signature         128 x 64-bit MinHash values
LSH layout        32 bands × 4 hashes
candidate gate    same LSH bucket
reject criterion  estimated Jaccard > 0.85
strict mode       exact Jaccard confirmation on candidate shingle hashes
```

The strict final comparison is worth doing because the user specified a hard Jaccard threshold. MinHash should locate candidates; exact set intersection should make the final decision.

The critical parallel portion is roughly:

```rust
use rayon::prelude::*;
use xxhash_rust::xxh3::xxh3_64_with_seed;

const NUM_HASHES: usize = 128;

#[derive(Clone)]
pub struct Signature {
    pub values: [u64; NUM_HASHES],
}

fn shingle_hashes(src: &[u8]) -> Vec<u64> {
    let lines: Vec<&[u8]> = src.split(|b| *b == b'\n').collect();

    lines.windows(5)
        .map(|window| {
            let mut h = 0xcbf29ce484222325u64;
            for line in window {
                h = xxh3_64_with_seed(line, h);
            }
            h
        })
        .collect()
}

fn minhash(shingles: &[u64]) -> Signature {
    let mut values = [u64::MAX; NUM_HASHES];

    for &shingle in shingles {
        for (i, value) in values.iter_mut().enumerate() {
            let candidate =
                xxh3_64_with_seed(&shingle.to_le_bytes(), i as u64);
            *value = (*value).min(candidate);
        }
    }

    Signature { values }
}

pub fn signatures(
    docs: &[AcceptedDocument],
) -> Vec<(usize, Signature)> {
    docs.par_iter()
        .enumerate()
        .map(|(idx, doc)| {
            let shingles = shingle_hashes(&doc.content);
            (idx, minhash(&shingles))
        })
        .collect()
}

fn minhash_similarity(a: &Signature, b: &Signature) -> f32 {
    let equal = a.values
        .iter()
        .zip(&b.values)
        .filter(|(x, y)| x == y)
        .count();

    equal as f32 / NUM_HASHES as f32
}
```

In the real implementation, retain only signatures globally. Recompute or temporarily cache exact shingle sets only for LSH candidate pairs. This prevents deduplication metadata itself from becoming a memory bottleneck.

Also deduplicate **by repository identity before global near-dedup** where possible. Otherwise forks and vendored copies can dominate the retained corpus.

### Tokenizer and binary representation

Hugging Face Tokenizers is implemented in Rust and supports training new vocabularies directly, including ByteLevel BPE, so it meets the zero-Python requirement. citeturn22search3turn22search6 For code, ByteLevel BPE is particularly attractive because indentation, punctuation, and otherwise-unusual byte sequences remain representable.

I would train the 32k tokenizer on exactly **2,000,000,000 scrubbed source bytes**, deterministically selected after deduplication. Do not lowercase source and do not normalize runs of whitespace: Python's indentation is syntax.

The important special tokens can remain minimal:

```text
<|endoftext|>      document boundary / EOS
<|pad|>            optional; avoid it during packed causal training
<|unk|>            normally almost irrelevant with byte-level fallback
```

The BPE training skeleton is:

```rust
use anyhow::Result;
use tokenizers::{
    Tokenizer,
    models::bpe::BPE,
    pre_tokenizers::byte_level::ByteLevel,
    trainers::bpe::BpeTrainer,
    AddedToken,
};

pub fn train_code_bpe<I>(
    texts: I,
    output: &str,
) -> Result<Tokenizer>
where
    I: Iterator<Item = String>,
{
    let bpe = BPE::builder()
        .unk_token("<|unk|>".into())
        .build()?;

    let mut tokenizer = Tokenizer::new(bpe);
    tokenizer.with_pre_tokenizer(Some(ByteLevel::default()));

    let special = vec![
        AddedToken::from("<|endoftext|>", true),
        AddedToken::from("<|pad|>", true),
        AddedToken::from("<|unk|>", true),
    ];

    let mut trainer = BpeTrainer::builder()
        .vocab_size(32_000)
        .min_frequency(2)
        .special_tokens(special)
        .show_progress(true)
        .build();

    tokenizer.train_from_iterator(texts, &mut trainer)?;
    tokenizer.save(output, false)?;

    Ok(tokenizer)
}
```

The exact method signatures should be locked against the selected `tokenizers` release through `Cargo.lock`; the underlying Rust-native training architecture is supported by Tokenizers' documented training pipeline. citeturn22search3turn22search9

For 32,000 vocabulary entries, token IDs fit comfortably into `u16`. Exactly 2 billion packed token IDs therefore require:

\[
2,000,000,000 \times 2
=4,000,000,000\ {\rm bytes}
\approx 3.725\ {\rm GiB}.
\]

I recommend eight roughly 500 MB shards rather than one monolithic object:

```text
tokens/
    shard-00000.u16
    shard-00001.u16
    ...
    shard-00007.u16

    corpus.json
    documents.idx
    tokenizer.json
```

`corpus.json` should record at least tokenizer SHA-256, token count, source snapshot/version, filtering configuration, seed, shard hashes, and endianness. `documents.idx` records token offsets of document boundaries.

`memmap2` currently exposes file-backed mappings directly as byte slices, so a token reader is simple. citeturn18search4turn18search13

```rust
use anyhow::{ensure, Result};
use memmap2::{Mmap, MmapOptions};
use std::{fs::File, path::Path};

pub struct TokenMmap {
    map: Mmap,
}

impl TokenMmap {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let map = unsafe { MmapOptions::new().map(&file)? };

        ensure!(map.len() % 2 == 0, "truncated u16 token file");
        Ok(Self { map })
    }

    pub fn len(&self) -> usize {
        self.map.len() / 2
    }

    pub fn token(&self, index: usize) -> u16 {
        let offset = index * 2;
        u16::from_le_bytes([
            self.map[offset],
            self.map[offset + 1],
        ])
    }

    pub fn copy_window(
        &self,
        start: usize,
        dst: &mut [u16],
    ) {
        for (i, x) in dst.iter_mut().enumerate() {
            *x = self.token(start + i);
        }
    }
}
```

A crucial correction is required here: **memory-mapping a token file does not make it zero-copy to a discrete RTX 5090 GPU.** It is zero-copy from the filesystem/page cache into the CPU address space; CUDA still needs host-to-device movement unless host pages are explicitly mapped/registered. CUDA documents `cudaMallocHost` for page-locked allocations and `cudaHostRegister` for registering existing host memory, and asynchronous host/device transfers require page-locked host memory for the intended overlap behavior. NVIDIA also cautions that excessive page-locked allocation reduces memory available to the operating system. citeturn5search11turn5search15turn6search1

Therefore, **do not pin the entire 3.7 GiB corpus mmap**. The fast design is:

```text
4-GB token mmap
       │
       ├── sampled 2048-token windows
       ▼
2–4 reusable 64–256 MB cudaMallocHost buffers
       │
       ├── cudaMemcpyAsync on transfer stream
       ▼
preallocated GPU batch buffers
       │
       └── compute stream
```

Similarly, Arrow does not expose a contract that Parquet decompression lands directly in a CUDA page-locked allocator. The sensible implementation is Arrow-managed decode followed by bounded filtering/tokenization, with page-locked memory introduced only at the final GPU staging boundary. That provides essentially all of the transfer benefit without pinning a large, churn-heavy CPU working set. Arrow's current Parquet interfaces directly support asynchronous decoding into Arrow batches. citeturn22search1turn22search13

## Windows MSVC and CUDA build configuration

For the RTX 5090, the CUDA 12.x floor matters. CUDA 12.8 introduced compiler support for Blackwell targets including **SM 120**, the architecture used by the consumer RTX 50-series path, so older CUDA 12.x installations should not be treated as sufficient for compiling native RTX 5090 kernels. citeturn7search3 Within the user's CUDA-12.x constraint, I would standardize on **CUDA 12.9 Update 1** or, at minimum, CUDA 12.8 rather than an arbitrary “CUDA 12.x”.

`nvcc` separates CUDA device compilation from host compilation and invokes an available host C++ compiler; on Windows the supported Microsoft compiler toolchain is the normal configuration. citeturn21search10turn0search1 The Rust `cc` crate supports CUDA compilation from a Cargo build script, making it suitable for an optional native-kernel bridge. citeturn17search0

Upstream FlashAttention-3 should **not** be wired into this project as the default. Its official beta path is specifically optimized for H100/H800 Hopper GPUs. citeturn0search3 Burn's own CubeCL Flash Attention is a better zero-Python first choice; if that cannot sustain the SLA, implement only the attention operation behind a C ABI using CUTLASS/CUDA. Current CUTLASS documentation supports CUDA 12.x and specifically recommends newer CUDA versions for its newest kernels. citeturn7search14

A reasonable project manifest is:

```toml
# Cargo.toml
[package]
name = "rust-llm-pretrain"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
build = "build.rs"

[features]
default = []
# Enables our own .cu/.cpp bridge. Burn/CubeCL does not require this feature.
native-kernels = []

[dependencies]
anyhow = "1"
async-trait = "0.1"
bytemuck = { version = "1", features = ["derive"] }
bytes = "1"
clap = { version = "4", features = ["derive"] }
futures-util = "0.3"

# Native Rust data path
arrow = "59.1.0"
parquet = { version = "59.1.0", features = ["arrow", "async"] }
memmap2 = "0.9.11"
rayon = "1.12"
reqwest = {
    version = "0.13.4",
    default-features = false,
    features = ["rustls", "stream", "json", "zstd", "gzip"]
}

# Syntax / tokenization
tree-sitter = "0.26.12"
tree-sitter-python = "0.25.0"
tokenizers = "0.23.1"
xxhash-rust = { version = "0.8", features = ["xxh3"] }

# Serialization / logging
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

tokio = {
    version = "1",
    features = ["rt-multi-thread", "macros", "fs", "sync", "signal"]
}

# Rust-native CUDA training stack
burn = "0.21.0"
burn-autodiff = "0.21.0"
burn-cuda = "0.21.0"

[build-dependencies]
cc = { version = "1", features = ["parallel"] }

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1

[profile.release.package."*"]
opt-level = 3
```

The listed current data-layer versions are consistent with the August 2026 crate documentation: Parquet 59.1 is the current native Arrow implementation, memmap2 0.9.11 is current as of June 2026, reqwest 0.13.4 is current, and Tree-sitter's current 0.26 line interoperates with the 0.25 Python grammar's `LANGUAGE` interface. citeturn18search6turn18search4turn19search4turn19search0turn19search1 Burn 0.21 was released in May 2026, while its CUDA backend documentation shows the direct `Autodiff<Cuda<f32, i32>>` style of backend composition. citeturn20search0turn25search13

The custom build script should be feature-gated because Burn/CubeCL can run without your own C++ object files:

```rust
// build.rs
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cuda/attention.cu");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    println!("cargo:rerun-if-env-changed=CUDNN_PATH");

    if env::var_os("CARGO_FEATURE_NATIVE_KERNELS").is_none() {
        return;
    }

    let target = env::var("TARGET").expect("TARGET missing");
    assert!(
        target == "x86_64-pc-windows-msvc",
        "native-kernels expects x86_64-pc-windows-msvc"
    );

    // Fail early instead of getting an opaque NVCC host-compiler error.
    let cl_ok = Command::new("where")
        .arg("cl.exe")
        .status()
        .map(|x| x.success())
        .unwrap_or(false);

    assert!(
        cl_ok,
        "cl.exe not found. Run Cargo from an x64 Developer PowerShell."
    );

    let cuda = PathBuf::from(
        env::var_os("CUDA_PATH")
            .expect("CUDA_PATH must point at CUDA 12.8+"),
    );

    let include = cuda.join("include");
    let cuda_lib = cuda.join("lib").join("x64");

    assert!(include.exists(), "CUDA include directory missing");
    assert!(cuda_lib.exists(), "CUDA lib/x64 directory missing");

    // NVCC performs CUDA device compilation and invokes MSVC for host code.
    let mut build = cc::Build::new();
    build
        .cuda(true)
        .cpp(true)
        .file("cuda/attention.cu")
        .include(&include)
        .flag("-O3")
        .flag("-std=c++17")
        .flag("-arch=sm_120")
        .flag("--use_fast_math")
        .flag("-lineinfo")
        .compile("rust_llm_cuda");

    link_search(&cuda_lib);

    // Import-library names; rustc resolves the .lib files on MSVC.
    link("cudart");
    link("cublas");
    link("cublasLt");
    link("curand");

    // cuDNN is optional for the Burn/CubeCL path but supported for
    // native fused kernels.
    if let Some(cudnn) = env::var_os("CUDNN_PATH") {
        let cudnn = PathBuf::from(cudnn);
        let lib = cudnn.join("lib").join("x64");

        if lib.exists() {
            link_search(&lib);
        }

        link("cudnn");
    }
}

fn link_search(path: &Path) {
    println!("cargo:rustc-link-search=native={}", path.display());
}

fn link(name: &str) {
    println!("cargo:rustc-link-lib=dylib={name}");
}
```

The C boundary should be **tiny**. Do not expose C++ classes, templates, or CUDA types to Rust:

```cpp
// cuda/attention.cu
#include <cuda_runtime.h>
#include <stdint.h>

extern "C" {

struct FlashGqaParams {
    const void* q;
    const void* k;
    const void* v;
    void* out;

    int32_t batch;
    int32_t seq;
    int32_t q_heads;
    int32_t kv_heads;
    int32_t head_dim;

    float scale;
    cudaStream_t stream;
};

// Return cudaError_t as an integer.
// Implementation can later be Cube/CUTLASS-derived Blackwell attention.
int rust_flash_gqa_bf16(const FlashGqaParams* p) noexcept {
    if (!p) {
        return static_cast<int>(cudaErrorInvalidValue);
    }

    // Native kernel dispatch goes here.
    //
    // In the first implementation this can deliberately return
    // cudaErrorNotSupported so Burn/CubeCL remains the fallback.
    return static_cast<int>(cudaErrorNotSupported);
}

}
```

And the Rust side remains ABI-stable:

```rust
#[repr(C)]
pub struct FlashGqaParams {
    pub q: *const core::ffi::c_void,
    pub k: *const core::ffi::c_void,
    pub v: *const core::ffi::c_void,
    pub out: *mut core::ffi::c_void,

    pub batch: i32,
    pub seq: i32,
    pub q_heads: i32,
    pub kv_heads: i32,
    pub head_dim: i32,

    pub scale: f32,
    pub stream: *mut core::ffi::c_void,
}

unsafe extern "C" {
    pub fn rust_flash_gqa_bf16(
        p: *const FlashGqaParams,
    ) -> i32;
}
```

This architecture keeps the project's **control plane, model, optimizer, dataset system, scheduling, and all high-level ML code in Rust**, while permitting CUDA kernels at exactly the layer where the GPU vendor toolchain is useful.

## Transformer architecture with GQA, RoPE, RMSNorm, and SwiGLU

The requested dimensions yield a head width of:

\[
d_h = \frac{768}{12} = 64.
\]

With four KV heads, each KV head services three query heads:

\[
{\rm kv\_head}(h_q)
=
\left\lfloor
\frac{h_q}{3}
\right\rfloor .
\]

The important tensor shapes per layer are therefore:

```text
input X       [B, L, 768]

Q projection  [B, L, 12, 64]
K projection  [B, L,  4, 64]
V projection  [B, L,  4, 64]

attention     Q-head 0,1,2   -> KV-head 0
              Q-head 3,4,5   -> KV-head 1
              Q-head 6,7,8   -> KV-head 2
              Q-head 9,10,11 -> KV-head 3

output        [B, L, 768]
```

The optimized kernel should **not physically repeat K and V three times**. It should derive the KV-head address from the query-head index. A generic fallback may expand them logically, but that unnecessarily increases memory traffic.

The RoPE frequency for pair index \(i\) is:

\[
\omega_i =
\theta^{-2i/d_h},\quad
\theta=10000,
\]

and, for token position \(p\),

\[
\begin{aligned}
x'_{2i} &=
x_{2i}\cos(p\omega_i)
-
x_{2i+1}\sin(p\omega_i),\\
x'_{2i+1} &=
x_{2i}\sin(p\omega_i)
+
x_{2i+1}\cos(p\omega_i).
\end{aligned}
\]

Apply this to **Q and K before attention**, never to V.

The requested block is:

```text
x
│
├─ RMSNorm
│    └─ Q/K/V
│       └─ RoPE(Q,K)
│       └─ causal GQA
│       └─ Wo
│
├─ residual add
│
├─ RMSNorm
│    └─ gate = W_gate x
│    └─ up   = W_up x
│    └─ SiLU(gate) * up
│    └─ W_down
│
└─ residual add
```

Burn 0.21 contains RMS normalization and rotary-position machinery in its neural-network/tensor ecosystem, while its current attention API has gained causal and attention-autotuning work. citeturn12search1turn12search2turn20search0

A framework-independent configuration should make the model invariants explicit:

```rust
#[derive(Clone, Debug)]
pub struct LlamaConfig {
    pub vocab_size: usize,
    pub d_model: usize,
    pub d_ff: usize,
    pub n_layers: usize,
    pub n_q_heads: usize,
    pub n_kv_heads: usize,
    pub max_seq_len: usize,
    pub rms_eps: f64,
    pub rope_theta: f64,
    pub tie_embeddings: bool,
}

impl LlamaConfig {
    pub fn strict_spec() -> Self {
        Self {
            vocab_size: 32_000,
            d_model: 768,
            d_ff: 2_048,
            n_layers: 12,
            n_q_heads: 12,
            n_kv_heads: 4,
            max_seq_len: 2_048,
            rms_eps: 1.0e-5,
            rope_theta: 10_000.0,
            tie_embeddings: false,
        }
    }

    // This is the configuration that actually lands at ~135M.
    pub fn budget_135m() -> Self {
        Self {
            d_ff: 2_432,
            ..Self::strict_spec()
        }
    }

    pub fn validate(&self) {
        assert_eq!(self.d_model % self.n_q_heads, 0);
        assert_eq!(self.n_q_heads % self.n_kv_heads, 0);
        assert!(self.vocab_size <= u16::MAX as usize);
    }

    pub fn head_dim(&self) -> usize {
        self.d_model / self.n_q_heads
    }

    pub fn kv_dim(&self) -> usize {
        self.n_kv_heads * self.head_dim()
    }

    pub fn q_per_kv(&self) -> usize {
        self.n_q_heads / self.n_kv_heads
    }
}
```

The projection dimensions should be constructed explicitly instead of letting a generic multi-head-attention layer accidentally instantiate 12 K and V heads:

```rust
// Conceptual Burn layer construction.
//
// q_proj:    768 -> 768
// k_proj:    768 -> 256
// v_proj:    768 -> 256
// out_proj:  768 -> 768
//
// gate_proj: 768 -> d_ff
// up_proj:   768 -> d_ff
// down_proj: d_ff -> 768

pub struct GqaShape {
    pub q_out: usize,
    pub kv_out: usize,
}

impl GqaShape {
    pub fn from_config(c: &LlamaConfig) -> Self {
        Self {
            q_out: c.n_q_heads * c.head_dim(),
            kv_out: c.n_kv_heads * c.head_dim(),
        }
    }
}
```

The attention operation itself should have semantics equivalent to:

```rust
pub fn kv_head_for_query(
    q_head: usize,
    n_q_heads: usize,
    n_kv_heads: usize,
) -> usize {
    debug_assert_eq!(n_q_heads % n_kv_heads, 0);

    let q_per_kv = n_q_heads / n_kv_heads;
    q_head / q_per_kv
}
```

A custom kernel can thus access:

```cpp
const int kv_head = q_head / 3;
```

rather than creating a `[B,12,L,64]` K tensor.

For a generic Burn fallback, expansion must preserve **three consecutive copies per KV head**:

```text
[B,4,L,64]
 -> [B,4,1,L,64]
 -> broadcast/repeat [B,4,3,L,64]
 -> reshape [B,12,L,64]
```

A naïve `repeat` of the complete head dimension can produce the wrong ordering, so the intermediate grouping dimension matters.

The feed-forward block is standard SwiGLU:

\[
{\rm FFN}(x)
=
W_{\rm down}
\left[
{\rm SiLU}(W_{\rm gate}x)
\odot
(W_{\rm up}x)
\right].
\]

Burn's neural-network layer set includes SwiGLU and RMSNorm components, so these do not require a Python-backed library. citeturn12search5

### Exact parameter accounting

With `d_ff=2048`:

| Component | Parameters |
|---|---:|
| Q projection / layer | 589,824 |
| K projection / layer | 196,608 |
| V projection / layer | 196,608 |
| O projection / layer | 589,824 |
| SwiGLU three matrices / layer | 4,718,592 |
| Two RMSNorm scales / layer | 1,536 |
| **Per layer** | **6,292,992** |
| Twelve layers | 75,515,904 |
| 32k × 768 embedding | 24,576,000 |
| Final RMSNorm | 768 |
| Untied LM head | 24,576,000 |
| **Total** | **124,668,672** |

Thus the requested architecture is not actually ~135M.

Keeping every other hyperparameter fixed and setting:

```text
d_ff = 2432
```

gives:

\[
135,285,504
\]

parameters with the untied 32k output head.

For reproducibility I would make the **124.67M strict model** and **135.29M corrected model** separate named presets rather than silently changing the user's architecture.

## Pre-training loop, VRAM management, and throughput engineering

### BF16 should be the primary training mode

The RTX 5090's Blackwell Tensor Cores natively provide high BF16 and FP8 throughput. citeturn10view0 Burn/CubeCL's CUDA path is designed around GPU kernel generation, fusion, memory management, and autotuning, and recent Burn releases specifically added Flash Attention plus additional attention autotuning. citeturn20search0turn25search0

The safest progression is therefore:

```text
Stage A     BF16 model/activations
            FP32 Adam moments
            Flash Attention
            native GQA kernel if available

Stage B     profile and prove ≥80k tokens/s

Stage C     selectively introduce FP8 GEMMs only if needed
            retain FP32 optimizer state
            validate loss against BF16 run
```

Do not make “everything FP8” the initial implementation. A corrupt or unstable eight-hour run costs more than the theoretical gain.

### Persistent memory is small; activations are the real constraint

For the 135.29M model, a conservative mixed-precision persistent state consists approximately of:

```text
BF16 parameters       2 bytes × 135.29M
BF16 gradients        2 bytes × 135.29M
FP32 Adam m + v       8 bytes × 135.29M
FP32 master weights   4 bytes × 135.29M
                     ───────────────────
Total                 ~2.02 GiB
```

The 124.67M variant is about 1.86 GiB under the same assumption.

This is why a 32 GB GPU can comfortably hold the model state, but **attention implementation is decisive**. A single dense BF16 attention-score tensor for batch 32 is approximately:

\[
32 \times 12 \times 2048 \times 2048 \times 2
\approx 3.0\ {\rm GiB}
\]

before considering gradients and twelve layers. Materializing standard attention matrices across the backward graph is therefore incompatible with the intended microbatch. Flash-style attention is effectively a requirement for the `B=32, L=2048` target, not merely a small optimization.

I would not intentionally fill “~28 GB” as a fixed target. Leave several gigabytes for kernel workspaces, JIT/autotuning choices, CUDA allocations, and fragmentation. A better operational criterion is:

```text
steady training allocation:        preferably <= 26–27 GiB
transient peak after warm-up:       must remain < 29 GiB
hard safety reserve:                approximately 3+ GiB
```

Those numbers are engineering guardrails rather than hardware guarantees and must be measured on the actual Windows machine.

### Batch configuration

The initial target should be:

```text
sequence length              2,048
microbatch                    32 sequences
tokens / microstep            65,536
gradient accumulation         2
tokens / optimizer update     131,072
optimizer updates / 2B        15,259
```

This is preferable to accumulation 8 for an additional reason: the requested 1,000-update warmup is then 6.55% of training instead of 26.2%.

| Microbatch | Accumulation | Global token batch | Approx. updates | 1,000-step warmup share |
|---:|---:|---:|---:|---:|
| 32 | 1 | 65,536 | 30,518 | 3.3% |
| **32** | **2** | **131,072** | **15,259** | **6.6%** |
| 32 | 4 | 262,144 | 7,630 | 13.1% |
| 32 | 8 | 524,288 | 3,815 | 26.2% |

If batch 32 does not fit once backward kernels are included, preserve approximately the same global token batch:

```text
B=32, accum=2       131,072 tokens/update
B=24, accum=3       147,456 tokens/update
B=16, accum=4       131,072 tokens/update
```

This is preferable to immediately enabling full-layer activation checkpointing. Checkpointing saves VRAM by recomputation, but that consumes exactly the GPU cycles needed to make the eight-hour target. Use it only if Flash Attention plus the smaller microbatch is insufficient.

### AdamW and schedule

Burn currently provides AdamW in its optimizer stack, allowing the optimizer itself to remain Rust-native. citeturn12search0 The required configuration is:

```rust
#[derive(Debug, Clone, Copy)]
pub struct TrainConfig {
    pub peak_lr: f64,
    pub warmup_steps: u64,
    pub beta1: f64,
    pub beta2: f64,
    pub weight_decay: f64,
    pub grad_clip_norm: f64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            peak_lr: 2.5e-3,
            warmup_steps: 1_000,
            beta1: 0.9,
            beta2: 0.95,
            weight_decay: 0.1,
            grad_clip_norm: 1.0,
        }
    }
}
```

The requested warmup/cosine scheduler can be independent of the ML framework:

```rust
pub fn learning_rate(
    step: u64,
    total_steps: u64,
    peak: f64,
    warmup: u64,
) -> f64 {
    if step < warmup {
        return peak * (step as f64 + 1.0) / warmup as f64;
    }

    let decay_steps = total_steps.saturating_sub(warmup).max(1);
    let progress =
        (step.saturating_sub(warmup) as f64 / decay_steps as f64)
        .clamp(0.0, 1.0);

    let cosine = 0.5 * (1.0 + (std::f64::consts::PI * progress).cos());
    peak * cosine
}
```

One optimizer detail should be implemented carefully: **weight decay should normally not be applied to RMSNorm scale parameters**. The model parameter registry should separate matrix weights from norm scales before constructing the AdamW parameter groups.

### Streaming trainer design

The loader should pre-generate offsets and stage batches without touching the training thread:

```text
Rayon sampling workers
       │
       ▼
pinned host ring [0] ──H2D──┐
pinned host ring [1] ──H2D──┤
pinned host ring [2] ──H2D──┤
                            ▼
                    GPU batch slot A/B
                            │
                            ▼
                    forward + backward
                            │
                  accum == target?
                      │           │
                     no          yes
                      │           │
                      └─────► AdamW update
```

The basic Rust orchestration can look like:

```rust
use std::time::{Duration, Instant};

pub struct ThroughputMeter {
    start: Instant,
    last: Instant,
    total_tokens: u64,
    window_tokens: u64,
}

impl ThroughputMeter {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            total_tokens: 0,
            window_tokens: 0,
        }
    }

    pub fn update(&mut self, tokens: u64) {
        self.total_tokens += tokens;
        self.window_tokens += tokens;

        let now = Instant::now();
        let dt = now.duration_since(self.last);

        if dt >= Duration::from_secs(10) {
            let instant_tps =
                self.window_tokens as f64 / dt.as_secs_f64();
            let overall_tps =
                self.total_tokens as f64 /
                now.duration_since(self.start).as_secs_f64();

            tracing::info!(
                tokens = self.total_tokens,
                window_tok_s = instant_tps,
                overall_tok_s = overall_tps,
                "training throughput"
            );

            self.window_tokens = 0;
            self.last = now;
        }
    }
}
```

The actual training state machine should conceptually be:

```rust
for update in 0..total_updates {
    optimizer.zero_grad();

    for micro in 0..grad_accum {
        let batch = loader.next_pinned_batch()?;

        // H2D was preferably started while the preceding microbatch computed.
        let x = batch.inputs_on_device();
        let y = batch.targets_on_device();

        // [B, 2048] -> [B, 2048, vocab]
        let logits = model.forward(x);

        // Fused causal cross-entropy preferred.
        let loss = causal_cross_entropy(logits, y)
            / grad_accum as f32;

        backward(loss);
        throughput.update(
            (micro_batch * seq_len) as u64
        );
    }

    clip_global_grad_norm(&model, 1.0);

    let lr = learning_rate(
        update,
        total_updates,
        2.5e-3,
        1_000,
    );

    optimizer.step(lr, &mut model);

    if should_checkpoint(update) {
        checkpoint_queue.try_enqueue(snapshot(&model, &optimizer))?;
    }
}
```

In Burn, autodiff should be layered onto the CUDA backend; the official CUDA-backend example uses `Autodiff<Cuda<...>>` for precisely this composition. citeturn25search13 For the finished implementation, use Burn's native gradient/optimizer parameter abstractions rather than literal `backward()` helper functions shown above.

### Throughput logging must measure the real workload

Do not report only CUDA-kernel time as “tokens/sec”. The eight-hour requirement is wall clock, so the primary metric must encompass:

```text
host batch preparation
+
H2D transfer
+
forward
+
loss
+
backward
+
optimizer
+
required synchronization
```

Also log:

```text
tok/s: instantaneous 10-second window
tok/s: rolling 5-minute average
tok/s: since-run-start
step time
loader wait time
H2D wait time
GPU allocated/peak bytes
loss
learning rate
gradient norm
tokens completed
predicted finish timestamp
```

The finish predictor is especially valuable:

```rust
pub fn eta_seconds(
    target_tokens: u64,
    completed_tokens: u64,
    rolling_tps: f64,
) -> f64 {
    target_tokens
        .saturating_sub(completed_tokens) as f64
        / rolling_tps.max(1.0)
}
```

At 2B tokens, the process should issue an explicit warning whenever the long-window rate falls below about 75k and an SLA failure warning below 70k.

Burn's recent releases include autotuning and reduced framework overhead, and its 0.20 release specifically introduced CubeCL Flash Attention, TMA autotuning, and MMA-oriented optimizations including work aimed at Blackwell-class GPUs. citeturn20search0 This makes it a credible Rust-native starting point, but those features do **not** establish that this exact 135M/2048/B32 workload will sustain 80k tok/s on Windows. That result needs a measured benchmark on the target machine.

## Repository layout and end-to-end implementation specification

I would structure the finished repository as follows:

```text
rust-llm-pretrain/
│
├── Cargo.toml
├── Cargo.lock
├── build.rs
│
├── cuda/
│   ├── attention.cu
│   └── attention.h
│
├── src/
│   ├── main.rs
│   │
│   ├── data/
│   │   ├── mod.rs
│   │   ├── source.rs
│   │   ├── parquet.rs
│   │   ├── stack_v2.rs
│   │   ├── filter.rs
│   │   ├── minhash.rs
│   │   ├── tokenizer.rs
│   │   ├── pack.rs
│   │   └── mmap.rs
│   │
│   ├── model/
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── rmsnorm.rs
│   │   ├── rope.rs
│   │   ├── gqa.rs
│   │   ├── swiglu.rs
│   │   └── llama.rs
│   │
│   ├── cuda/
│   │   ├── mod.rs
│   │   ├── ffi.rs
│   │   └── pinned.rs
│   │
│   └── train/
│       ├── mod.rs
│       ├── loader.rs
│       ├── optimizer.rs
│       ├── schedule.rs
│       ├── checkpoint.rs
│       └── metrics.rs
│
├── tokenizer/
│   └── tokenizer.json
│
├── corpus/
│   ├── corpus.json
│   ├── documents.idx
│   ├── shard-00000.u16
│   └── ...
│
└── checkpoints/
```

The CLI should make every stage independently restartable:

```text
rust-llm-pretrain curate
    source manifests/content
    -> AST filtering
    -> quality filter
    -> deduped source manifest

rust-llm-pretrain train-tokenizer
    scrubbed source
    -> deterministic 2GB subset
    -> tokenizer.json

rust-llm-pretrain tokenize
    scrubbed source + tokenizer.json
    -> 2B u16 tokens
    -> document index
    -> corpus metadata

rust-llm-pretrain bench
    token mmap
    -> 200–500 warmup microsteps
    -> measured B16/B24/B32 configurations

rust-llm-pretrain train
    corpus + tokenizer + config
    -> checkpoints
    -> metrics.jsonl
```

This is entirely compatible with a no-Python operational environment: Tokenizers' core implementation is Rust, Arrow/Parquet has a native Rust implementation, Tree-sitter has a native Rust API and Python grammar, and Burn/CubeCL supplies the Rust CUDA training path. citeturn22search3turn18search6turn19search0turn25search0

The curation pipeline should additionally preserve a content hash and provenance entry for every retained document. This is particularly important with The Stack v2 because the dataset explicitly preserves provenance for source-code licensing and receives removal updates. citeturn21search0 It also makes deduplication deterministic and allows a corpus to be regenerated after removals without trying to reverse-map anonymous token offsets.

The build/run environment should be fixed before the eight-hour run:

```text
Windows 11 x64
Rust stable >= project MSRV
target: x86_64-pc-windows-msvc

Visual Studio / Build Tools:
    x64 MSVC compiler
    Windows SDK

CUDA:
    CUDA 12.9 U1 recommended
    CUDA 12.8 minimum for this Blackwell target
    CUDA_PATH set

cuDNN:
    compatible Windows package
    CUDNN_PATH set only if native cuDNN path is enabled

GPU:
    RTX 5090 32 GB
```

CUDA 12.8's addition of Blackwell compilation support is the reason for the lower bound, rather than merely choosing a recent toolkit arbitrarily. citeturn7search3

The final performance qualification should happen in three gates.

**Correctness gate.** Train approximately 10–50 million tokens, verify finite loss/gradients, compare generic GQA and optimized GQA outputs at FP32/BF16 tolerances, verify checkpoint/reload identity, and ensure the tokenizer plus mmap corpus round-trip correctly.

**Memory gate.** Run at least several hundred microsteps at sequence 2048. Start at B16, then B24, then B32. Record the post-autotune steady-state and peak allocation. Do not accept B32 merely because its first forward pass fits.

**SLA gate.** Execute a sufficiently long run to include data loading, backward, optimizer steps, logging, and at least one checkpoint. The requirement should be **≥80k wall-clock tokens/sec over a multi-minute window**, not an isolated forward-pass benchmark. The 70k figure is too close to the absolute mathematical minimum to constitute a robust eight-hour design target.

The recommended final configuration is therefore:

| System element | Production choice |
|---|---|
| Host | Windows 11, `x86_64-pc-windows-msvc` |
| CUDA | **12.9 U1**, minimum 12.8 for Blackwell target |
| GPU | RTX 5090, 32 GB GDDR7 citeturn21search2 |
| Rust ML backend | **Burn 0.21 + burn-cuda/CubeCL** citeturn20search0turn25search13 |
| Attention | CubeCL Flash Attention first; native SM120/CUTLASS fallback |
| FA3 | **Do not use as RTX 5090 default**; upstream FA3 targets Hopper citeturn0search3 |
| Precision | **BF16 first**, selective FP8 only after BF16 benchmark |
| Model | 12 × 768, 12 Q / 4 KV, head dim 64 |
| Strict `d_ff=2048` model | **124.67M parameters** |
| True ~135M model | `d_ff=2432`, **135.29M parameters** |
| Context | 2048 |
| Tokenizer | Rust ByteLevel BPE, vocab 32k citeturn22search3 |
| Corpus | 2B little-endian `u16` IDs, ~3.725 GiB |
| Corpus access | `memmap2` + bounded pinned staging, not whole-mmap pinning citeturn18search4turn6search1 |
| Base microbatch | 32 × 2048 |
| Accumulation | **2** initially |
| Global token batch | 131,072 |
| Optimizer | AdamW, β₁=.9, β₂=.95, wd=.1 |
| Schedule | 1,000-step linear warmup → cosine decay |
| Runtime break-even | 69.44k tok/s |
| Engineering SLA | **80–85k tok/s sustained** |
| Eight-hour scope | **pretraining after corpus materialization** |

The decisive technical choices are therefore not FP8 or artificially maximizing allocated VRAM. They are **native-GQA Flash Attention, avoiding physical KV repetition, a bounded mmap→pinned→GPU pipeline, BF16 Tensor-Core GEMMs, keeping the optimizer path fused/low-overhead, and making the training batch large enough to amortize kernel launches**. The RTX 5090 has sufficient theoretical BF16 compute and 1.792 TB/s memory bandwidth to make the experiment plausible, but only a target-machine benchmark can turn that plausibility into the claimed eight-hour result. citeturn10view0turn21search2