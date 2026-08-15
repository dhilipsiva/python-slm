//! Pinned, zero-Python Tree-sitter boundary for Python source policy.

use crate::data::ByteRange;
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tree_sitter::{Language, Node, ParseOptions, ParseState, Parser, Point};

pub const IMPLEMENTATION_PHASE: &str = "P5";
pub const PARSER_RESULT_SCHEMA: &str = "python-slm-python-parser-result-v1";
pub const PARSER_BUNDLE_SCHEMA: &str = "python-slm-parser-bundle-binding-v1";
pub const PARSER_IDENTITY_SCHEMA: &str = "python-slm-parser-identity-v1";
pub const TREE_SITTER_VERSION: &str = "0.25.8";
pub const TREE_SITTER_LANGUAGE_VERSION: &str = "0.1.7";
pub const TREE_SITTER_PYTHON_VERSION: &str = "0.25.0";
pub const PYTHON_LANGUAGE_ABI: usize = 15;
pub const MINIMUM_LANGUAGE_ABI: usize = 13;
pub const MAXIMUM_CST_DEPTH: u64 = 4_096;

const IDENTITY_BYTES: &[u8] = include_bytes!("parser/tree-sitter-python-0.25.0.identity.json");
const CST_DIGEST_DOMAIN: &[u8] = b"python-slm/cst/v1\0";
const LEXICAL_DIGEST_DOMAIN: &[u8] = b"python-slm/lexical-tokens/v1\0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserPackageIdentity {
    pub name: String,
    pub version: String,
    pub crates_io_checksum: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserSourceIdentity {
    pub package: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserIdentity {
    pub packages: Vec<ParserPackageIdentity>,
    pub language_name: String,
    pub language_abi_version: u64,
    pub minimum_compatible_language_abi: u64,
    pub normalized_build: String,
    pub compatibility_path: String,
    pub compatibility_sha256: String,
    pub sources: Vec<ParserSourceIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserIdentityManifest {
    pub schema: String,
    pub identity: ParserIdentity,
    pub bundle_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserBundleBinding {
    pub schema: &'static str,
    pub tree_sitter_version: &'static str,
    pub tree_sitter_language_version: &'static str,
    pub tree_sitter_python_version: &'static str,
    pub language_abi_version: u64,
    pub bundle_sha256: String,
    pub compatibility_sha256: String,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexicalToken {
    pub kind: String,
    pub text: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PythonParserResult {
    pub schema: &'static str,
    pub parser_bundle: ParserBundleBinding,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub status: &'static str,
    pub reasons: Vec<&'static str>,
    pub policy_status: &'static str,
    pub policy_reason: Option<&'static str>,
    pub root_kind: Option<String>,
    pub root_start_byte: Option<u64>,
    pub root_end_byte: Option<u64>,
    pub has_error: Option<bool>,
    pub node_count: Option<u64>,
    pub maximum_depth: Option<u64>,
    pub cst_sha256: Option<String>,
    pub cst_sexp_sha256: Option<String>,
    pub lexical_token_count: Option<u64>,
    pub lexical_tokens_sha256: Option<String>,
    pub comment_ranges: Vec<ByteRange>,
    pub comment_bytes: u64,
    pub work_budget: u64,
    pub work_callbacks: u64,
    pub node_limit: u64,
    pub depth_limit: u64,
}

impl PythonParserResult {
    pub fn apply_policy(&mut self, accepted: bool, reason: Option<&'static str>) {
        self.policy_status = if accepted { "ACCEPTED" } else { "REJECTED" };
        self.policy_reason = reason;
        if let Some(reason) = reason {
            self.status = "REJECTED";
            self.reasons.push(reason);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedPython {
    pub result: PythonParserResult,
    pub lexical_tokens: Vec<LexicalToken>,
}

pub fn identity_manifest() -> Result<ParserIdentityManifest> {
    let manifest: ParserIdentityManifest =
        serde_json::from_slice(IDENTITY_BYTES).map_err(|_| {
            ProductError::internal(
                "PARSER_IDENTITY_INVALID",
                "the embedded parser identity manifest is malformed",
            )
        })?;
    if manifest.schema != PARSER_IDENTITY_SCHEMA
        || manifest.identity.language_name != "python"
        || manifest.identity.language_abi_version != PYTHON_LANGUAGE_ABI as u64
        || manifest.identity.minimum_compatible_language_abi != MINIMUM_LANGUAGE_ABI as u64
        || !is_lower_sha256(&manifest.bundle_sha256)
        || !is_lower_sha256(&manifest.identity.compatibility_sha256)
    {
        return Err(ProductError::integrity(
            "PARSER_IDENTITY_INVALID",
            "the embedded parser identity manifest violates the pinned contract",
        ));
    }
    let identity_bytes = serde_json::to_vec(&manifest.identity).map_err(|_| {
        ProductError::internal(
            "PARSER_IDENTITY_INVALID",
            "the embedded parser identity could not be serialized",
        )
    })?;
    if sha256(&identity_bytes) != manifest.bundle_sha256 {
        return Err(ProductError::integrity(
            "PARSER_IDENTITY_HASH_MISMATCH",
            "the embedded parser identity bundle hash does not match",
        ));
    }
    Ok(manifest)
}

pub fn parser_bundle_binding() -> Result<ParserBundleBinding> {
    let manifest = identity_manifest()?;
    Ok(ParserBundleBinding {
        schema: PARSER_BUNDLE_SCHEMA,
        tree_sitter_version: TREE_SITTER_VERSION,
        tree_sitter_language_version: TREE_SITTER_LANGUAGE_VERSION,
        tree_sitter_python_version: TREE_SITTER_PYTHON_VERSION,
        language_abi_version: PYTHON_LANGUAGE_ABI as u64,
        bundle_sha256: manifest.bundle_sha256,
        compatibility_sha256: manifest.identity.compatibility_sha256,
    })
}

pub fn parse_python(
    canonical_bytes: &[u8],
    cancellation: &CancellationToken,
) -> Result<ParsedPython> {
    let parser_bundle = parser_bundle_binding()?;
    let source_bytes = u64::try_from(canonical_bytes.len()).map_err(|_| {
        ProductError::integrity("PARSER_INPUT_INVALID", "the parser input length overflowed")
    })?;
    let work_budget = source_bytes
        .checked_mul(64)
        .and_then(|value| value.checked_add(4_096))
        .ok_or_else(|| {
            ProductError::integrity("PARSER_INPUT_INVALID", "the parser work budget overflowed")
        })?;
    let node_limit = source_bytes
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            ProductError::integrity("PARSER_INPUT_INVALID", "the parser node limit overflowed")
        })?;
    if cancellation.is_cancelled() {
        return Err(ProductError::gate(
            "PARSER_CANCELLED",
            "the parser operation was cancelled before parsing",
        ));
    }

    let language: Language = tree_sitter_python::LANGUAGE.into();
    if language.name() != Some("python")
        || language.abi_version() != PYTHON_LANGUAGE_ABI
        || !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&language.abi_version())
    {
        return Err(ProductError::integrity(
            "PARSER_ABI_MISMATCH",
            "the pinned Python grammar is incompatible with the pinned parser runtime",
        ));
    }
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|_| {
        ProductError::integrity(
            "PARSER_ABI_MISMATCH",
            "the pinned Python grammar could not be installed",
        )
    })?;

    let mut work_callbacks = 0_u64;
    let mut budget_exhausted = false;
    let mut externally_cancelled = false;
    let tree = {
        let mut progress = |_state: &ParseState| {
            work_callbacks = work_callbacks.saturating_add(1);
            externally_cancelled = cancellation.is_cancelled();
            budget_exhausted = work_callbacks > work_budget;
            externally_cancelled || budget_exhausted
        };
        let options = ParseOptions::new().progress_callback(&mut progress);
        let mut input = |offset: usize, _point: Point| &canonical_bytes[offset..];
        parser.parse_with_options(&mut input, None, Some(options))
    };
    if externally_cancelled {
        return Err(ProductError::gate(
            "PARSER_CANCELLED",
            "the parser operation was cancelled before completion",
        ));
    }
    if budget_exhausted {
        return Ok(ParsedPython {
            result: empty_rejection(
                parser_bundle,
                canonical_bytes,
                work_budget,
                work_callbacks,
                node_limit,
                "PARSER_WORK_BUDGET_EXCEEDED",
            ),
            lexical_tokens: Vec::new(),
        });
    }
    let tree = tree.ok_or_else(|| {
        ProductError::internal(
            "PARSER_NO_TREE",
            "the pinned parser returned no tree without cancellation",
        )
    })?;
    let root = tree.root_node();
    let traversal = traverse(root, canonical_bytes, node_limit)?;
    let mut reasons = Vec::new();
    if root.kind() != "module"
        || root.start_byte() != 0
        || root.end_byte() != canonical_bytes.len()
        || traversal.has_error
        || traversal.has_python2_syntax
    {
        reasons.push("PYTHON_SYNTAX_REJECTED");
    }
    if traversal.node_limit_exceeded {
        reasons.push("PARSER_NODE_LIMIT_EXCEEDED");
    }
    if traversal.maximum_depth > MAXIMUM_CST_DEPTH {
        reasons.push("PARSER_DEPTH_LIMIT_EXCEEDED");
    }
    reasons.sort_unstable();
    reasons.dedup();
    let lexical_tokens = lexical_tokens(root, canonical_bytes)?;
    let comment_bytes = traversal
        .comment_ranges
        .iter()
        .try_fold(0_u64, |total, range| {
            let bytes = u64::try_from(range.end - range.start).map_err(|_| {
                ProductError::integrity("PARSER_FACTS_INVALID", "comment byte count overflowed")
            })?;
            total.checked_add(bytes).ok_or_else(|| {
                ProductError::integrity("PARSER_FACTS_INVALID", "comment byte count overflowed")
            })
        })?;
    let cst_sexp = root.to_sexp();
    let status = if reasons.is_empty() {
        "PARSER_ACCEPTED"
    } else {
        "REJECTED"
    };
    Ok(ParsedPython {
        result: PythonParserResult {
            schema: PARSER_RESULT_SCHEMA,
            parser_bundle,
            source_sha256: sha256(canonical_bytes),
            source_bytes,
            status,
            reasons,
            policy_status: "NOT_EVALUATED",
            policy_reason: None,
            root_kind: Some(root.kind().to_owned()),
            root_start_byte: Some(root.start_byte() as u64),
            root_end_byte: Some(root.end_byte() as u64),
            has_error: Some(traversal.has_error),
            node_count: Some(traversal.node_count),
            maximum_depth: Some(traversal.maximum_depth),
            cst_sha256: Some(traversal.cst_sha256),
            cst_sexp_sha256: Some(sha256(cst_sexp.as_bytes())),
            lexical_token_count: Some(lexical_tokens.len() as u64),
            lexical_tokens_sha256: Some(lexical_digest(&lexical_tokens)?),
            comment_ranges: traversal.comment_ranges,
            comment_bytes,
            work_budget,
            work_callbacks,
            node_limit,
            depth_limit: MAXIMUM_CST_DEPTH,
        },
        lexical_tokens,
    })
}

fn empty_rejection(
    parser_bundle: ParserBundleBinding,
    source: &[u8],
    work_budget: u64,
    work_callbacks: u64,
    node_limit: u64,
    reason: &'static str,
) -> PythonParserResult {
    PythonParserResult {
        schema: PARSER_RESULT_SCHEMA,
        parser_bundle,
        source_sha256: sha256(source),
        source_bytes: source.len() as u64,
        status: "REJECTED",
        reasons: vec![reason],
        policy_status: "NOT_EVALUATED",
        policy_reason: None,
        root_kind: None,
        root_start_byte: None,
        root_end_byte: None,
        has_error: None,
        node_count: None,
        maximum_depth: None,
        cst_sha256: None,
        cst_sexp_sha256: None,
        lexical_token_count: None,
        lexical_tokens_sha256: None,
        comment_ranges: Vec::new(),
        comment_bytes: 0,
        work_budget,
        work_callbacks,
        node_limit,
        depth_limit: MAXIMUM_CST_DEPTH,
    }
}

struct Traversal {
    node_count: u64,
    maximum_depth: u64,
    has_error: bool,
    has_python2_syntax: bool,
    node_limit_exceeded: bool,
    cst_sha256: String,
    comment_ranges: Vec<ByteRange>,
}

fn traverse(root: Node<'_>, source: &[u8], node_limit: u64) -> Result<Traversal> {
    let mut stack = vec![(root, 0_u64, u32::MAX, None)];
    let mut hasher = Sha256::new();
    hasher.update(CST_DIGEST_DOMAIN);
    let mut node_count = 0_u64;
    let mut maximum_depth = 0_u64;
    let mut has_error = false;
    let mut has_python2_syntax = false;
    let mut comment_ranges = Vec::new();
    while let Some((node, depth, child_index, field_name)) = stack.pop() {
        node_count = node_count.checked_add(1).ok_or_else(|| {
            ProductError::integrity("PARSER_FACTS_INVALID", "parser node count overflowed")
        })?;
        maximum_depth = maximum_depth.max(depth);
        has_error |= node.is_error() || node.is_missing();
        has_python2_syntax |= is_python2_only_node(node);
        update_lp(&mut hasher, node.kind().as_bytes())?;
        hasher.update((node.start_byte() as u64).to_le_bytes());
        hasher.update((node.end_byte() as u64).to_le_bytes());
        hasher.update(child_index.to_le_bytes());
        hasher.update([
            u8::from(node.is_named()),
            u8::from(node.is_extra()),
            u8::from(node.is_error()),
            u8::from(node.is_missing()),
        ]);
        update_lp(&mut hasher, field_name.unwrap_or("").as_bytes())?;
        if node.kind() == "comment" {
            comment_ranges.push(ByteRange {
                start: node.start_byte(),
                end: node.end_byte(),
            });
        }
        for index in (0..node.child_count()).rev() {
            let child = node.child(index).ok_or_else(|| {
                ProductError::integrity(
                    "PARSER_FACTS_INVALID",
                    "the parser child inventory changed during traversal",
                )
            })?;
            let child_index = u32::try_from(index).map_err(|_| {
                ProductError::integrity("PARSER_FACTS_INVALID", "parser child index overflowed")
            })?;
            stack.push((
                child,
                depth.saturating_add(1),
                child_index,
                node.field_name_for_child(child_index),
            ));
        }
    }
    comment_ranges.sort_by_key(|range| (range.start, range.end));
    let mut previous_end = 0;
    for range in &comment_ranges {
        if range.start >= range.end || range.start < previous_end || range.end > source.len() {
            return Err(ProductError::integrity(
                "PARSER_FACTS_INVALID",
                "parser comment ranges are invalid",
            ));
        }
        previous_end = range.end;
    }
    Ok(Traversal {
        node_count,
        maximum_depth,
        has_error,
        has_python2_syntax,
        node_limit_exceeded: node_count > node_limit,
        cst_sha256: hex::encode(hasher.finalize()),
        comment_ranges,
    })
}

fn is_python2_only_node(node: Node<'_>) -> bool {
    matches!(node.kind(), "print_statement" | "exec_statement")
        || (node.kind() == "except_clause"
            && (0..node.child_count())
                .any(|index| node.child(index).is_some_and(|child| child.kind() == ",")))
}

fn lexical_tokens(root: Node<'_>, source: &[u8]) -> Result<Vec<LexicalToken>> {
    let mut stack = vec![root];
    let mut tokens = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "comment" {
            continue;
        }
        if node.is_named() && matches!(node.kind(), "identifier" | "string" | "integer" | "float") {
            tokens.push(token(node, source)?);
            continue;
        }
        if node.child_count() == 0 {
            let bytes = source.get(node.byte_range()).ok_or_else(|| {
                ProductError::integrity("PARSER_FACTS_INVALID", "a parser leaf is out of bounds")
            })?;
            if !bytes.is_empty() && !bytes.iter().all(u8::is_ascii_whitespace) {
                tokens.push(LexicalToken {
                    kind: node.kind().to_owned(),
                    text: bytes.to_vec(),
                });
            }
            continue;
        }
        for index in (0..node.child_count()).rev() {
            stack.push(node.child(index).ok_or_else(|| {
                ProductError::integrity(
                    "PARSER_FACTS_INVALID",
                    "the parser child inventory changed during lexical traversal",
                )
            })?);
        }
    }
    Ok(tokens)
}

fn token(node: Node<'_>, source: &[u8]) -> Result<LexicalToken> {
    let text = source.get(node.byte_range()).ok_or_else(|| {
        ProductError::integrity("PARSER_FACTS_INVALID", "a parser token is out of bounds")
    })?;
    Ok(LexicalToken {
        kind: node.kind().to_owned(),
        text: text.to_vec(),
    })
}

fn lexical_digest(tokens: &[LexicalToken]) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(LEXICAL_DIGEST_DOMAIN);
    for token in tokens {
        update_lp(&mut hasher, token.kind.as_bytes())?;
        update_lp(&mut hasher, &token.text)?;
    }
    Ok(hex::encode(hasher.finalize()))
}

fn update_lp(hasher: &mut Sha256, bytes: &[u8]) -> Result<()> {
    let length = u64::try_from(bytes.len()).map_err(|_| {
        ProductError::integrity("PARSER_FACTS_INVALID", "parser digest field overflowed")
    })?;
    hasher.update(length.to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
