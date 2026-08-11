# Rebuild Contract

Status: **AWAITING_REVIEW**

Contract version: `rebuild-contract-v1`

Frozen against source commit: `4354a4ec5cefdb2c7b462562991a33686969778e`

Frozen source tree: `eb400714687589e6a7e6a4395b5d43bb6333501a`

Decision date: 2026-08-11

## Status, Authority, and Compatibility

This document freezes the product and acceptance contract for a clean rebuild. Repository
instructions govern how work is performed, `docs/ARCHITECTURE.md` governs the target design,
this contract resolves Phase 0 product choices, and later accepted ADRs may refine only the
qualification choices explicitly deferred below. A conflict is a stop condition; an agent must
not silently choose a precedence or infer a new requirement.

The architecture's packaging paths are illustrative. `CLI-001` refines that single point: the
public product is one installed executable whose subcommands are independently restartable.
TODO P3's phrase "independently restartable binaries" therefore means independently invocable
subcommands, not multiple installed executables. This does not constrain internal command
modules or import the reference module layout.

The existing `src/`, `build.rs`, root configurations, and artifacts are behavioral evidence
only. No implementation, dependency selection, module structure, serialization schema, or
backend integration is inherited. In the disposition matrix:

- **KEEP** means preserve an observable behavior through an independent implementation.
- **REIMPLEMENT** means preserve an intent but replace or extend its public contract.
- **DROP** means the observable behavior is incompatible with the rebuild.

Compatibility is limited to useful stage names and explicitly retained behavior. There is no
promise that old flags, configuration files, corpus files, token shards, checkpoints, Rust APIs,
or error text will remain readable.

## Terminology

- **Stored ID:** one immutable `u16le` token identifier in a governed training artifact.
- **Consumed input:** an ID presented as model input; this is not automatically a valid target.
- **Valid predicted target:** an unmasked next-token label included in loss, optimizer-batch, and
  SLA accounting.
- **Boundary exclusion:** a real position inside the contracted training prefix omitted from
  valid-target accounting by a boundary policy. Unused tail and runtime padding are separate
  categories; EOS transitions are not exclusions.
- **Unused tail:** governed IDs beyond the exact contracted training prefix.
- **Microstep:** one forward/backward accumulation unit containing its actual valid-target count.
- **Optimizer update:** one AdamW step after one or more microsteps; the final update may be
  smaller than the configured full update.
- **Checkpoint generation:** one atomically published, self-verifying, fully resumable state.
- **Prepared corpus:** immutable, materialized training and held-out artifacts supplied before
  the SLA clock starts; the trainer opens and re-verifies them after the clock starts.
- **Padding input/target:** runtime-only alignment positions; they are never stored corpus IDs
  and their targets are always masked.
- **Training prefix:** the first 2,000,000,001 IDs in the canonical packed training stream; it
  exposes exactly 2,000,000,000 adjacent input/target transitions.

## Public Interface and Failure Contract

The rebuilt product is one `python-slm` executable with independently restartable `plan`,
`curate`, `train-tokenizer`, `tokenize`, `inspect`, `bench`, and `train` subcommands. Each
mutating production stage requires an explicit versioned JSON configuration. Schemas contain a
`schema_version`, reject unknown fields, validate cross-artifact identities, and have no hidden
production defaults. Named presets are `gqa-135m-v1` and `gqa-124m-ref-v1`.

On success, stdout contains exactly one terminal JSON object using
`python-slm-stage-receipt-v1`; stderr may contain UTF-8 JSONL diagnostics. On a handled failure,
stdout is empty and stderr is UTF-8 JSONL whose final non-empty line is a
`python-slm-error-v1` object. Human-only free-form diagnostics are not emitted by production
commands. OS termination, power loss, or a native abort can prevent a terminal object; callers
detect that case from the OS/nonzero status plus the missing terminal receipt. Long-running
structured telemetry is written to an explicitly configured JSONL artifact; the terminal
success receipt records its artifact-root-relative path and SHA-256. No channel may contain
credentials, raw source, PII, or secret values. The handled-error object's minimum shape is:

```json
{
  "schema": "python-slm-error-v1",
  "category": "integrity",
  "code": "ARTIFACT_HASH_MISMATCH",
  "message": "artifact verification failed",
  "context": {}
}
```

Exit categories are fixed: `0` success, `1` unexpected internal failure, `2` usage or
configuration failure, `3` policy or artifact-integrity failure, `4` environment or external-I/O
failure, and `5` qualification-gate failure. Per-document curation rejections are reason-coded
stage results, not process failures. Expected external/configuration failures return typed errors
rather than panicking. Production artifact verification is mandatory and cannot be disabled.

## Reference Disposition Matrix

| Area | Reference evidence | Disposition | Rebuild contract |
|---|---|---|---|
| Pipeline stages | `src/main.rs::Command` | KEEP | Preserve independently runnable stage concepts; add `inspect` and `bench`. |
| JSON success output | `PlanReport`, `print_json` | KEEP | Emit a versioned machine-readable receipt on stdout. |
| Current binary/module dispatch | `src/main.rs`, `src/lib.rs` | DROP | Implement the `python-slm` CLI and module boundaries from the target architecture. |
| Parameter/SLA and bounded-FLOP arithmetic | `LlamaConfig`, `PlanReport` | KEEP | Independently reproduce exact parameter/SLA arithmetic, explicitly bounded FLOP estimates with exclusions, and qualification status. |
| Default 124M identity | `LlamaConfig::default` | DROP | The exact 135M preset is canonical; 124M is explicitly reference-only. |
| Configuration schemas | `src/config.rs`, data config structs | REIMPLEMENT | Version, reject unknown fields, and make policy/artifact identities explicit. |
| Shape/range validation | `LlamaConfig::validate`, `TrainConfig::validate` | KEEP | Preserve and extend positive-size, divisibility, vocabulary, and optimizer checks. |
| Production training gate | `enforce_optimized_kernel_gate` | KEEP | Production remains fail-closed; correctness paths are separately named smoke modes. |
| Optional hash checking | `--verify-hashes`, `TokenDataset::open` | DROP | Always verify complete manifest and artifact hash chains before use. |
| Nominal token counting | `tokens_per_micro_step`, `total_optimizer_steps` | DROP | Count actual valid targets and execute an exact final partial update with zero overshoot. |
| Reference cosine endpoint | `TrainConfig::learning_rate` | DROP | Use the exact endpoint rule in `SCHED-001`. |
| HTTPS, checksums, credentials | `RemoteManifest`, `download_one` | REIMPLEMENT | Make HTTPS, complete hash chains, bounds, and environment-only credentials mandatory in production. |
| Generic plain HTTP | `DownloadConfig::allow_plain_http` | DROP | Permit HTTP only inside an explicit local-fixture mode with address restrictions. |
| Parquet/Stack distinction | `curate_remote_parquet`, `README.md` | KEEP | Preserve the fact that metadata/IDs and content-bearing Parquet are distinct sources. |
| Authorized Stack-v2/SWH adapter | No complete reference adapter | REIMPLEMENT | Retrieve governed content only through authorized, provenance-preserving access. |
| Download-all flow | `download_manifest`, `curate_remote_parquet` | DROP | Use bounded streaming, backpressure, retries, and resumable verified generations. |
| Parse/allocation bounds | Parquet and corpus checks | REIMPLEMENT | Preserve bounded-resource intent while applying the new 1,000,000-byte document limit and per-stage budgets. |
| Curation policy | `PythonFilter`, `CurationStats` | REIMPLEMENT | Add governed encoding, license, removal, sensitive-data, split, and reason contracts. |
| In-memory 128-MinHash dedup | `MINHASH_SIZE`, `DedupIndex` | DROP | Use the disk-backed, 256-component, qualified design in `DEDUP-001..003`. |
| First-input duplicate winner | `insert_if_unique` | DROP | Use the deterministic representative order in `DEDUP-001`. |
| Local-path source identity | Parquet extraction | DROP | Derive stable IDs from governed provenance, never host paths. |
| Flat `RLCORP02` corpus | `src/data/corpus.rs` | DROP | Use governed raw/decoded/curated generations and manifests. |
| Create-new publication | corpus and shard writers | KEEP | Preserve refusal to overwrite an existing named output. |
| Atomic generation finalization | incomplete in reference writers | REIMPLEMENT | Use same-volume partial generations, durability sync, manifest-last publication, and crash recovery. |
| Tokenizer intent | `train_bpe`, `SPECIAL_TOKENS` | REIMPLEMENT | Qualify capped sampling, exact IDs, byte round-trip, repeat hashes, and atomic artifacts. |
| `u16le` token foundation | `TokenManifest`, `validate_manifest` | REIMPLEMENT | Add global offsets, indexes, all provenance/config hashes, and generation finalization. |
| Corpus-end wrap | `TokenDataset::batch` | DROP | Never create a tail-to-head label or duplicate a span. |
| Stored-token target | `TokenizeConfig::target_tokens` | DROP | Materialize enough IDs to expose exactly the contracted valid targets. |
| Pageable mmap description | `TokenDataset` comments | KEEP | Keep mmap explicitly pageable; transfer selection remains measured. |
| Burn `.mpk` checkpoint | `save_model_checkpoint` | DROP | Store the full resumable state defined by `CKPT-001`. |
| Training telemetry | `TrainingSummary` | REIMPLEMENT | Report valid-target counters, p50/p95, stalls, evaluation, checkpoint time, VRAM, and hashes. |
| Expected failure behavior | validation and gate errors | REIMPLEMENT | Return stable typed categories, fail closed, and never panic for expected input/environment failures. |
| CPU/CUDA isolation | Cargo feature gates | REIMPLEMENT | CPU/data builds must not discover, compile, or link CUDA; GPU qualification remains separate. |
| Reference test intentions | inline `#[cfg(test)]` modules | KEEP | Independently rewrite count, filter, dedup, tokenizer, causal, GQA, gradient, schedule, allocation, footer, accumulation, format, hash, atomicity, and VRAM-gate tests. |
| 22 tests as qualification | `VALIDATION.md` | DROP | Treat them only as Phase 0 evidence, not rebuild/GPU/performance proof. |

## Frozen Decision Ledger

### Scope, SLA, and Interfaces

| ID | Frozen decision | Rationale and evidence | Reopen trigger |
|---|---|---|---|
| `SCOPE-001` | Windows-native Rust control plane; no Python interpreter, package, or subprocess in build, data, training, or qualification. Feature-gated CUDA/C ABI and Tree-sitter native code are allowed. | Matches the architecture's testable zero-Python boundary. | Only an owner-approved contract revision. |
| `SLA-001` | After corpus materialization, the 28,800-second clock starts immediately before the trainer opens frozen artifacts. Artifact-integrity verification, startup, model initialization, JIT/autotuning incurred by the run, data loading, all forward/backward/optimizer work, configured evaluation, synchronization, checkpointing, final durability, and any recovery downtime count. Completion is exactly 2,000,000,000 valid targets plus a durable final checkpoint, with zero overshoot. | `docs/ARCHITECTURE.md`; reference plans establish only the 69,444.444444... arithmetic floor. | SLA or target-count product change. |
| `CLI-001` | One `python-slm` CLI exposes `plan`, `curate`, `train-tokenizer`, `tokenize`, `inspect`, `bench`, and `train`. | Fewer packaging surfaces while retaining restartable stages. | Demonstrated ownership/build problem and ADR. |
| `COMPAT-001` | Preserve command concepts only; old flags, JSON, artifacts, checkpoints, Rust APIs, and backend behavior have no compatibility promise. | Avoids importing known semantic and format defects. | Explicit migration requirement from the owner. |
| `CONFIG-001` | Per-stage JSON schemas are versioned, reject unknown fields, and require explicit production settings and artifact identities. | Prevents silent defaults and configuration drift. | Schema-versioned migration ADR. |
| `ERROR-001` | Use the stdout/stderr, error schema, and exit categories defined above; expected failures never panic. | Enables stable automation and fail-closed operation. | Versioned CLI protocol change. |

### Data, Governance, and Sampling

| ID | Frozen decision | Rationale and evidence | Reopen trigger |
|---|---|---|---|
| `SOURCE-001` | Primary source is The Stack v2 Python metadata plus authorized Software Heritage content. Governed content-bearing Parquet is an explicit alternate adapter, never a hidden substitution. | Stack-v2 records identifiers rather than a simple content column. | Source-access failure plus named owner approval of an alternate. |
| `SOURCE-002` | Let `lp(x)=u64le(length(UTF8(x))) || UTF8(x)`. Every adapter has a versioned ASCII namespace and every record has an immutable provider record ID that is unique within `(adapter_namespace, source_snapshot_id)`; an adapter must extend a non-unique upstream ID with stable provenance fields before hashing. Set `source_id` to lowercase hex of `SHA256("python-slm/source-id/v1\0" || lp(adapter_namespace) || lp(source_snapshot_id) || lp(provider_record_id))`. If a provider repository ID exists, set `repository_group_id` to lowercase hex of `SHA256("python-slm/repository-group/v1\0" || lp(adapter_namespace) || lp(provider_repository_id))`. Otherwise use lowercase hex of `SHA256("python-slm/repository-group-fallback/v1\0" || lp(adapter_namespace) || lp(source_snapshot_id) || lp(stable_provenance_origin_namespace))`; absent narrower provenance uses the empty origin namespace, conservatively grouping the whole adapter snapshot. Accept strict UTF-8/ASCII Python 3 under `tree-sitter-python 0.25.0`. Recognize a PEP 263 cookie only on physical line one or two. Allow a leading UTF-8 BOM only with no cookie or a UTF-8-equivalent cookie; allow missing, UTF-8, UTF8, UTF-8-SIG, ASCII, or US-ASCII declarations. Reject invalid bytes, NULs, conflicting declarations, and all other encodings. BOM removal is recorded decode canonicalization. | Deterministic IDs, decoding, and grouping without replacement characters, semantic cookie rewriting, premature provenance collapse, or optimistic repository leakage. | Grammar/encoding/grouping fixture ADR and artifact-version change. |
| `SOURCE-003` | Apply size to canonical decoded bytes: accept `100..=1,000,000`. Sum the non-overlapping full byte ranges of all Tree-sitter `comment` nodes and reject iff `2 * comment_node_bytes > canonical_decoded_byte_length`; equality passes and docstrings are not comments. For generated markers, inspect each comment node's byte intersection with canonical range `[0,8192)` independently; do not concatenate nodes or scan strings. Match ASCII-case-insensitive substrings from registry `generated-v1`: `@generated`, `auto-generated`, `autogenerated`, `automatically generated`, `code generated`, `generated by`, `generated file`, `this file is generated`, `do not edit`. A shebang or encoding cookie participates exactly when Tree-sitter emits it as a comment node. | Bounded parsing and lower generated-data risk without ambiguous header or string scanning. | Yield/false-positive evidence and contract revision. |
| `GOV-001` | License allowlist `permissive-v1` is `0BSD`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `BSL-1.0`, `ISC`, `MIT`, `MIT-0`, `Python-2.0`, and `Zlib`. An SPDX OR expression passes only if one complete branch is allowlisted; every term of an AND branch must pass. Missing, unknown, `LicenseRef`, conflicting, copyleft, exception-bearing, or other terms fail. | Conservative, auditable rights boundary chosen by the owner. | Named data-governance approval and versioned policy revision. |
| `GOV-002` | Policy `sensitive-v1` rejects the whole training document on a confirmed private key, provider credential, credentialed URL, high-entropy secret assigned to a secret-like name, personal email outside reserved example domains, telephone number, government identifier, payment-account identifier, or confirmed postal address. Uncertain cases are quarantined. Raw governed evidence is restricted; no sensitive value enters logs/receipts and no tokenizer-visible redaction is permitted. Before P5 implementation, a reviewed ADR must freeze and hash registry `sensitive-rules-v1`, including exact patterns, validators, normalization, entropy calculation/thresholds, reserved domains, precedence, and quarantine rules. | The owner chose reject/quarantine over source rewriting; detector mechanics require measured, reviewable fixtures rather than agent invention. | Named governance approval and full downstream rerun. |
| `GOV-003` | The authoritative removal inputs are official Stack-v2 removal metadata and any applicable Software Heritage takedown/removal source named by the authorized adapter. Each adapter manifest records authority URL, provider snapshot ID, publication/retrieval times, and SHA-256. Pin the newest provider-ordered snapshot from every required authority during materialization and recheck within 24 hours before qualification and final training. Missing, unverifiable, or unorderable required snapshots stop the phase; a superseding mandatory removal blocks downstream work and rebuilds affected artifacts. | Prevents knowingly training a stale governed snapshot while defining who may supersede it. | Source provider changes its removal contract. |
| `TOKSAMPLE-001` | Select only deduplicated, decontaminated training documents. Encode every variable UTF-8 identifier as `u64le(byte_length) || bytes`. Rank whole documents by `SHA256("python-slm/tokenizer-sample/v1\0" || lp(repository_group_id) || lp(source_id) || curated_sha256_raw)`, ascending. Admit a document only if its canonical bytes keep both the repository-group cap at or below 10,000,000 and the global cap at or below 2,000,000,000; skip non-fitting records and continue. Train on separate documents in rank order without inserted specials. Qualified bytes must be `1,999,000,000..=2,000,000,000`; never split or duplicate a document. | Deterministic broad sampling with unambiguous serialization and a bounded whole-document gap. | Insufficient approved yield or contract revision. |
| `SPLIT-001` | Decontaminate first, then form connected components across repository-group identity and duplicate clusters. UTF-8 member IDs are byte-sorted and encoded as `u64le(byte_length) || bytes`; `component_id = SHA256("python-slm/component/v1\0" || encoded_members)`. Let `b = u64be(SHA256("python-slm/split/v1\0" || component_id)[0..8]) mod 10,000`: train iff `b < 9800`, validation iff `9800 <= b < 9900`, and test iff `9900 <= b < 10000`. | Deterministic 98/1/1 grouping without repository or duplicate leakage. | Split algorithm/version change requiring retokenization. |
| `DECONTAM-001` | Registry [`evalplus-v0.3.1`](https://github.com/evalplus/evalplus/releases/tag/v0.3.1) pins commit `e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2`, full `HumanEvalPlus.jsonl.gz` release asset `v0.1.10`, and the real full [`MbppPlus.jsonl.gz` release asset `v0.2.0`](https://github.com/evalplus/mbppplus_release/tags). The v0.3.1 notes' MBPP+ `v0.2.1` label has no corresponding release asset, while the pinned loader source selects `v0.2.0`; no nonexistent asset is invented. P6A verifies downloaded bytes before strict gzip/UTF-8/JSONL decoding, rejects duplicate JSON keys, and records asset/decoded hashes without executing Python. Sort records by raw UTF-8 `task_id`; within each record extract, in order: `/prompt`; `/prompt || /canonical_solution` as the full solution; present string `/contract`; present string `/test`; then each present string in `/test_list` and `/challenge_test_list` by array index. Normalize CRLF/CR to LF but do not trim or add bytes; manifest identity is dataset, task ID, JSON Pointer/index, pre/post hash, and role. Prompt and full-solution records must parse as standalone pinned Python. For a contract/test fragment that does not, lexical matching uses `def __evalplus_fragment__():\n` plus every physical fragment line indented four spaces and one wrapper-final LF, then removes every wrapper-introduced token before comparison. Present `/base_input` and `/plus_input` values are additionally protected for exact matching as RFC 8785 canonical JSON bytes, not lexical matching. Canonical exact source match means curated-byte equality, indexed by SHA-256. Jaccard and exact contiguous 50-lexical-token protected spans use `DEDUP-001`'s normalized lexical-token sequence and token encoding. For a protected lexical record shorter than 50 tokens, require complete sequence equality. Exclude on exact equality, exact Jaccard strictly above `0.85`, or a protected-span match. | Removes whole and substantial partial benchmark copies using real pinned extraction and one lexical representation. | Registry-version ADR and complete downstream artifact rebuild. |

### Deduplication, Tokenization, and Storage

| ID | Frozen decision | Rationale and evidence | Reopen trigger |
|---|---|---|---|
| `DEDUP-001` | Exact curated SHA-256 dedup runs first. For near-dedup, traverse the pinned Tree-sitter CST in increasing `(start_byte,end_byte,child_index)` order. Skip every `comment` subtree. Emit `identifier`, `string`, `integer`, and `float` named nodes atomically without descending. Otherwise descend non-leaves; for each nonempty leaf, discard it only when its exact bytes are all ASCII whitespace, else emit its grammar kind and exact bytes. Thus anonymous keyword/operator/delimiter leaves remain tokens and string/number contents stay atomic. Encode a token as `u64le(kind_utf8_length) || kind_utf8 || u64le(text_length) || exact_text_bytes`. Similarity is set Jaccard over consecutive lexical-token 5-gram byte strings. Exact and qualifying near-duplicate edges form transitive connected components independent of ingestion order. Every component retains every provenance/license record; representative priority is complete required provenance, lower exact comment-byte ratio, higher lexical-token count, then lexicographically smaller UTF-8 source ID. | Detects textual/lexical clones without conflating renamed programs or losing governance records. | Labeled-suite evidence plus contract revision. |
| `DEDUP-002` | For each canonical shingle byte string `s`, compute `x = u64le(SHA256("python-slm/shingle-base/v1\0" || len(s)_le64 || s)[0..8]) mod P`, where `P=2^61-1`. For component `i=0..255`, let `d_i=SHA256("python-slm/minhash-coeff/v1\0" || i_le32)`, `a_i=1+(u64le(d_i[0..8]) mod (P-1))`, `b_i=u64le(d_i[8..16]) mod P`, and `h_i(s)=(a_i*x+b_i) mod P` using exact `u128` intermediates. The signature is `sig[i]=min_{s in S} h_i(s)`. A document with fewer than five lexical tokens uses one `"short/v1\0" || token_count_le64 || encoded_token_sequence` shingle for both exact Jaccard and MinHash, including the zero-token sequence. LSH uses 32 bands of 8 consecutive signatures; its full key is `SHA256("python-slm/lsh-band/v1\0" || band_index_le32 || eight_u64le_signatures)`. Equal full keys retrieve candidates only. Exact shingle-set Jaccard rejects strictly above `0.85`; exactly `0.85` passes. | One SHA-256 per shingle plus deterministic universal-hash permutations gives a feasible, defined candidate index with exact final decisions. | P6 failure, ADR, P0 revision, and downstream rerun. |
| `DEDUP-003` | Qualification suite specification `dedup-threshold-v1` contains exactly 10,000 deterministic valid-Python pairs: 2,500 with exact Jaccard `<=0.84`, 2,500 at `0.85`, 2,500 in `(0.85,0.86]`, and 2,500 at `>=0.95`. Its versioned generator uses rational set intersections, fixed grammatical template families (functions, classes, branches, loops, comprehensions, exceptions, async, pattern matching, literals, and decorators), deterministic mutation enumeration, and seed `SHA256("python-slm/dedup-threshold-v1")`. P6 must materialize and human-review the generator/source hashes, pair manifest, and exhaustive exact truth before any LSH implementation, benchmark, or tuning; failure to fill every stratum stops P6. Require end-to-end recall `>=0.995` and final precision `1.0`; report candidate amplification separately. Once sealed, any generator, corpus, strata, seed, or suite-hash change creates a new suite version and contract revision. | Prevents tuning to an invented fixture while making LSH false negatives and exact precision visible. | Suite-version or threshold change. |
| `TOKEN-001` | Byte-level BPE explicitly seeds all 256 byte symbols, applies no case folding, Unicode normalization, or whitespace stripping, uses minimum merge frequency 2, and produces exactly 32,000 contiguous IDs with maximum ID 31,999 and zero unknown IDs on supported source. With special-token injection disabled, `decode(encode(bytes))` must equal the original curated UTF-8 bytes exactly. Two clean builds over one ordered manifest must be byte-identical. | Deterministic, whitespace-preserving Python tokenization. | Tokenizer algorithm/version ADR and full retokenization. |
| `TOKEN-002` | Fixed IDs are `<pad>=0`, `<s>=1`, `</s>=2`, `<unk>=3`. Never inject BOS. Source encoding bypasses special-token matching; only boundary/alignment code may emit IDs `0..3`. Append exactly one EOS after every document, including the final document. PAD exists only in masked runtime alignment; UNK is invalid for supported bytes. Samples may cross EOS and EOS-to-next-document targets remain valid. | Simple packed training with explicit boundaries and no literal-source special ambiguity. | Model/data semantic contract revision. |
| `ARTIFACT-001` | Token storage is immutable `u16le` shards plus versioned JSON manifests and document/sequence indexes. Every reader verifies schema, lengths, IDs, path containment, tokenizer/config/source/split hashes, and shard hashes before read-only mapping. Writers use unique same-volume partial generations, sync data and metadata, and atomically publish without overwrite. A 2,048-target full sample uses 2,049 logical IDs and never wraps at corpus end. | Portable integrity and exact shifted-label semantics. | Versioned format migration. |
| `ACCOUNT-001` | Order retained training representatives by component ID, repository-group ID, source ID, then curated SHA-256, all ascending by raw bytes. Encode each complete document and append one EOS. Materialization stops after completing the first document whose EOS makes the cumulative count at least 2,000,000,001. The first 2,000,000,001 IDs are the contracted training prefix; only the remainder of that completed document is stored unused tail. Later approved documents are not materialized and are separately counted as `unmaterialized_documents` and canonical bytes. `total_stored_ids = training_prefix_ids + unused_tail_ids`; within the prefix, input roles are IDs `0..1,999,999,999`, target roles are IDs `1..2,000,000,000`, and the last ID is the one shift anchor. Thus consumed real inputs and valid targets are each 2,000,000,000, boundary exclusions are zero, and EOS transitions are ordinary. The final 2,048-position aligned span contains exactly 1,024 real inputs/targets plus 1,024 PAD inputs/masked padding targets. Padding is non-stored. Every receipt reports and reconciles every named counter without overlap inside a role ledger. | Removes corpus-end wrap, unbounded tail materialization, nominal accounting, and ambiguity about which governed IDs are trained. | Packing/boundary/target-count revision and full downstream rerun. |

### Model and Training

| ID | Frozen decision | Rationale and evidence | Reopen trigger |
|---|---|---|---|
| `MODEL-001` | Preset `gqa-135m-v1`: vocabulary 32,000; width 768; FFN 2,432; 12 layers; 12 query and 4 KV heads; head width 64; context 2,048; untied LM head; no biases; exactly 135,285,504 parameters. | Analytical architecture and reference `--gqa-135m` agree. | Owner-approved model-version change. |
| `MODEL-002` | Preset `gqa-124m-ref-v1` differs only by FFN width 2,048 and has exactly 124,668,672 parameters. It is a reference/experiment preset, never the canonical default. | Prevents the original 124M/135M ambiguity. | Removal of the reference preset. |
| `PRECISION-001` | BF16 parameters and activations; FP32 accumulation or reductions for BF16 GEMMs, RMSNorm, softmax, loss, and gradients; FP32 Adam moments and master weights. Reduced results are cast back to the specified BF16 activation layout where applicable. FP8 is not the baseline. | Lowest-risk correctness baseline from the architecture. | Later isolated parity-qualified experiment. |
| `ROPE-001` | RoPE base is 10,000, pairs are adjacent `(2i, 2i+1)`, positions are `[0,2048)`, and positions reset at each packed sample start, not at EOS. | Removes backend-layout ambiguity. | Model semantic version change. |
| `MASK-001` | Inclusive lower-triangular causal mask: query `i` may attend valid key `j` iff `j <= i`. Padding is always masked. Packed documents do not add an attention reset at EOS. | Matches the selected packed-EOS policy and reference causal test intent. | Segment-aware model/data contract revision. |
| `INIT-001` | For both named presets, matrix and embedding weights use `Normal(0,0.02)`; norm scales initialize to one; dropout is zero; no residual-specific scaling is applied. Initialization order is `tok_embeddings.weight`; then for numeric block `i=0..11`: `attn_norm`, attention `q,k,v,o`, `ffn_norm`, FFN `gate,up,down`; then `final_norm.weight` and `lm_head.weight`, using `PARAM-001` names and row-major element order. Seed `rand_chacha 0.10.0` `ChaCha12Rng` with the 32 raw bytes of `SHA256("python-slm/init/v1\0" || UTF8(preset_name))`; sample an `f32` from `rand_distr 0.6.0` `StandardNormal`, multiply by the IEEE-754 binary32 value produced by round-to-nearest-even conversion of decimal `0.02`, and convert to BF16 round-to-nearest-even. P9B hashes each canonical initialized parameter artifact; every backend parity run uses that artifact. | Explicit reproducible baseline independent of backend registry order. | Qualified initialization experiment and model-version change. |
| `PARAM-001` | Stable names are `tok_embeddings.weight`; `blocks.{i}.attn_norm.weight`; `blocks.{i}.attn.{q,k,v,o}.weight`; `blocks.{i}.ffn_norm.weight`; `blocks.{i}.ffn.{gate,up,down}.weight`; `final_norm.weight`; and `lm_head.weight`. GQA maps query head `q` to KV head `q/3`. | Stable optimizer/checkpoint mapping independent of backend registry order. | Versioned checkpoint migration. |
| `OPT-001` | AdamW uses beta1 `0.9`, beta2 `0.95`, epsilon `1e-8`, weight decay `0.1`, and FP32 global-L2 gradient clipping at `1.0` across every trainable parameter. Unscale first; any non-finite gradient or norm fails the update. Let clip scale be `1` for zero norm, else `min(1, 1/norm)`. For one-based update `t`, after clipping: `m=beta1*m+(1-beta1)*g`; `v=beta2*v+(1-beta2)*g*g`; `mhat=m/(1-beta1^t)`; `vhat=v/(1-beta2^t)`; `theta=theta-lr*(mhat/(sqrt(vhat)+eps)+wd*theta)`. Use `wd=0.1` for embedding, LM-head, attention, and FFN matrices and `wd=0` for norm scales. Update FP32 master weights, then cast BF16 round-to-nearest-even. Normalize accumulated loss by actual valid targets; clip once, step once, and zero once per optimizer update. | Fixes bias correction, epsilon placement, decay order, grouping, clipping scope, and accumulation semantics. | Training-recipe ADR and new qualification identity. |
| `BATCH-001` | A full optimizer update contains 65,536 valid predicted targets. Microbatch and accumulation are measured later but must preserve this total. | Chosen optimization/update balance. | Artifact-hashed training experiment and downstream rerun. |
| `TAIL-001` | Exactly 2,000,000,000 valid targets produce 30,517 full updates and one final 37,888-target update: 30,518 updates total. The final update is loss-normalized by 37,888 and does not duplicate or overshoot. | Exact integer accounting. | Target or global-batch change. |
| `SCHED-001` | With one-based update `s` and total `N=30,518`: for `1 <= s <= 1,000`, `lr=0.0025*s/1000`; otherwise `p=(s-1000)/(N-1000)` and `lr=0.00025 + 0.5*(0.0025-0.00025)*(1+cos(pi*p))`. Thus update 1 is `2.5e-6`, update 1,000 is `2.5e-3`, and update 30,518 is `2.5e-4`. Scheduler advances only on optimizer updates, including the final partial update. | Exact warmup and endpoint convention. | Training-recipe ADR and new qualification identity. |
| `SPAN-001` | The exact contracted training prefix forms 976,562 complete 2,048-target spans and one final 1,024-target span; later stored IDs are unused tail. Shuffle only complete spans with descending Fisher-Yates. Seed `rand_chacha 0.10.0` `ChaCha12Rng` with `SHA256("python-slm/span-order/v1\0" || contract_decisions_sha256_raw32 || corpus_manifest_sha256_raw32)`. For each `i` from `n-1` through `1`, set `range=i+1` and `threshold=0u64.wrapping_sub(range) % range`; repeatedly set `x=rng.next_u64()` until `x >= threshold`, then swap `i` with `x % range`. Keep the partial span last for the alignment in `ACCOUNT-001`. Preserve token order inside spans and never sample a span twice. `contract_decisions_sha256_raw32` hashes the exact UTF-8/LF byte range beginning with `## Frozen Decision Ledger\n` and ending immediately before `\n## Deferred Qualification Facts`; manifest hashes are decoded from lowercase hex to raw 32-byte operands. | Deterministic resume-friendly order, status-independent contract identity, and exact final accounting. | Sampler-version change requiring new corpus/training identity. |
| `EVAL-001` | Build the validation packed stream with `ACCOUNT-001` document ordering. Enumerate its non-overlapping 2,048-target spans by global target offset and sort ascending by raw 32-byte `SHA256("python-slm/eval-sample/v1\0" || validation_split_manifest_sha256_raw32 || offset_u64le)`, breaking a digest tie by smaller offset. Select without replacement in that order: 488 full spans (999,424 targets) plus the first 576 targets of the 489th ranked span, for exactly 1,000,000 valid targets; insufficient validation targets stop materialization. Hash the ordered index manifest. Evaluate it before the first update and at the first completed optimizer boundary crossing each 100,000,000-target threshold, including completion. Evaluation does not advance training RNG, cursor, optimizer, or scheduler. Its wall time is inside the SLA. | Comparable fixed trend data with defined selection, order, and bounded overhead. | Evaluation-policy ADR and new qualification identity. |
| `CKPT-001` | At the same post-update thresholds and at completion, atomically save model, FP32 master/moment state, optimizer/scheduler/scaler, host/device RNG, sampler order/cursor, exact counters, configurations, and artifact hashes. Version 1 forbids mid-update checkpoints; every checkpoint is at a completed optimizer boundary after update/evaluation state is coherent. Coincident completion and 100M-threshold triggers emit one generation. Retain the latest two plus first generations at or after 500M, 1B, 1.5B, and final 2B targets. | Full restartability with bounded retained storage. | Checkpoint schema/retention ADR and resume requalification. |
| `PROV-001` | Every final receipt records approved full-contract and decision-ledger hashes; frozen source-tree hash; Git commit and dirty status; `Cargo.lock` SHA-256; toolchain, driver, GPU, and SM; backend/kernel build and configuration hashes; tokenizer, source/corpus/split manifests and source/removal approvals; telemetry/log and benchmark hashes; peak VRAM; loss/evaluation diagnostics; elapsed time; and final checkpoint hash. It separately records total stored IDs, 2,000,000,001 training-prefix IDs, unused-tail IDs, unmaterialized documents/bytes, consumed real inputs, padding inputs, valid targets, masked padding targets, boundary exclusions, optimizer-update counts, and checkpoint/evaluation counters; all equations in `ACCOUNT-001` must reconcile exactly. | Reproducible acceptance identity and non-overlapping accounting within each role ledger. | Receipt-schema version change. |

## Deferred Qualification Facts

These are measurements or external facts, not Phase 0 choices:

| Fact | Owning phase | P0 constraint |
|---|---|---|
| CPU/MSVC and CUDA/SM120 environment | P1A/P1B | Record; do not weaken target requirements. |
| Backend and numerical tolerances | P2, P10, P12 | Predeclare tolerances before each measured gate. |
| Authorized source access, snapshot identity, yield, and artifact hashes | P4, P6A | No hidden source substitution or self-approval. |
| Sensitive detector registry behavior and accepted/quarantined yield | P5 | Freeze `sensitive-rules-v1` in a reviewed ADR before implementation; preserve reject/quarantine policy. |
| Dedup suite bytes/hash, measured recall/precision, and candidate amplification | P6 | Meet `DEDUP-003`; do not claim exhaustive LSH recall. |
| EvalPlus imported artifact hashes | P6A | Match `DECONTAM-001` identities without executing Python. |
| Tokenizer vocabulary, repeat-build determinism, round-trip, and overflow qualification | P7 | Meet `TOKEN-001/002`; do not infer success from a single artifact. |
| Exact stored-ID count and unused tail | P9A | Must reconcile to exactly two billion valid targets. |
| Microbatch and accumulation pair | P14 | Preserve 65,536 valid targets per full update. |
| Pageable versus pinned transfer | P11 | Select only by end-to-end measurement. |
| Loss stability of the frozen schedule | P12 | Measure; recipe remains unqualified until finite/tiny-overfit and staged loss gates pass. |
| Evaluation/checkpoint time and retained-storage cost | P12, P14 | Include measured overhead in the SLA projection. |
| VRAM, throughput, and eight-hour feasibility | P10-P16 | Remain unverified until target-host gates pass. |

## Change Control

After status becomes `APPROVED`, changing a frozen decision requires an ADR, a
`rebuild-contract-vN` version increment, an artifact/schema impact review, invalidation of
affected receipts, and rerun of the owning phase plus every affected downstream gate. Changing
a hash, seed, boundary, threshold, source policy, target count, model shape, or training recipe
creates a new qualification identity. Corrections made while status is `AWAITING_REVIEW` remain
version 1 but invalidate and regenerate Phase 0 evidence. An agent cannot approve governance or
mark its own generated contract `PASS`.

## Phase 0 Coverage

| TODO requirement | Contract decision |
|---|---|
| Zero-Python scope and SLA | `SCOPE-001`, `SLA-001` |
| Canonical and reference models | `MODEL-001`, `MODEL-002` |
| Target-count semantics and zero overshoot | `SLA-001`, `ACCOUNT-001`, `TAIL-001`, `SPAN-001` |
| Special IDs and boundary injection | `TOKEN-002`, `MASK-001`, `ROPE-001` |
| Tokenizer budget and repository cap | `TOKSAMPLE-001` |
| Encoding, dialect, size, comments, generated markers | `SOURCE-002`, `SOURCE-003` |
| License, PII/secrets, removals | `GOV-001`, `GOV-002`, `GOV-003` |
| Splits and benchmark decontamination | `SPLIT-001`, `DECONTAM-001` |
| Dedup normalization, MinHash/LSH, qualification | `DEDUP-001`, `DEDUP-002`, `DEDUP-003` |
| RoPE, mask, initialization, parameter names | `ROPE-001`, `MASK-001`, `INIT-001`, `PARAM-001` |
| Optimizer, decay groups, scheduler | `OPT-001`, `SCHED-001` |
| Global valid-target batch and final update | `BATCH-001`, `TAIL-001` |
| Packed stream, span order, and seed | `ACCOUNT-001`, `SPAN-001` |
| Evaluation cadence | `EVAL-001` |
| Checkpoint cadence and retention | `CKPT-001` |
| Provenance and final receipt | `PROV-001`, `GOV-003` |
| CLI/config/artifact/failure compatibility matrix | `CLI-001`, `COMPAT-001`, `CONFIG-001`, `ERROR-001`, `ARTIFACT-001` |

Upon human approval, no Phase 0 product decision is left for an implementer to infer. Facts
listed under deferred qualification remain explicitly unverified rather than guessed, and their
owning phases must supply the specified ADRs, fixtures, hashes, or measurements before work that
depends on them begins.
