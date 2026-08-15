use crate::error::{ProductError, Result};
use crate::parser::CancellationToken;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const SENSITIVE_POLICY_ID: &str = "sensitive-v1";
pub const SENSITIVE_REGISTRY_ID: &str = "sensitive-rules-v1";
pub const SENSITIVE_RESULT_SCHEMA: &str = "python-slm-sensitive-policy-result-v1";
pub const SENSITIVE_BINDING_SCHEMA: &str = "python-slm-sensitive-policy-binding-v1";
const REGISTRY_SHA256: &str = "e6805ddf162b0d2ad3e43567328db76cd8aacf62167980e18140a4c36fed0658";

const REGISTRY_BYTES: &[u8] = include_bytes!("sensitive-rules-v1.json");
const PRIVATE_KEY_MARKERS: &[&[u8]] = &[
    b"-----BEGIN PRIVATE KEY-----",
    b"-----BEGIN RSA PRIVATE KEY-----",
    b"-----BEGIN EC PRIVATE KEY-----",
    b"-----BEGIN DSA PRIVATE KEY-----",
    b"-----BEGIN OPENSSH PRIVATE KEY-----",
    b"-----BEGIN PGP PRIVATE KEY BLOCK-----",
];
const STREET_SUFFIXES: &[&str] = &[
    "avenue",
    "boulevard",
    "circle",
    "court",
    "drive",
    "highway",
    "lane",
    "parkway",
    "place",
    "road",
    "street",
    "terrace",
    "trail",
    "way",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntropyPolicy {
    pub minimum_length: usize,
    pub maximum_length: usize,
    pub minimum_millibits_per_byte: u64,
    pub minimum_character_classes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivePolicyRegistry {
    pub schema: String,
    pub policy_id: String,
    pub registry_id: String,
    pub normalization: String,
    pub entropy: EntropyPolicy,
    pub reserved_email_domains: Vec<String>,
    pub secret_like_names: Vec<String>,
    pub confirmed_rule_ids: Vec<String>,
    pub quarantine_rule_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivePolicyBinding {
    pub schema: &'static str,
    pub policy_id: &'static str,
    pub registry_id: &'static str,
    pub registry_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivePolicyResult {
    pub schema: &'static str,
    pub policy: SensitivePolicyBinding,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub status: &'static str,
    pub reason: Option<&'static str>,
    pub rule_ids: Vec<String>,
    pub finding_count: u64,
    pub restricted_values_emitted: bool,
    pub source_rewritten: bool,
}

pub fn policy_registry() -> Result<SensitivePolicyRegistry> {
    let registry: SensitivePolicyRegistry =
        serde_json::from_slice(REGISTRY_BYTES).map_err(|_| {
            ProductError::internal(
                "SENSITIVE_REGISTRY_INVALID",
                "the embedded sensitive-data registry is malformed",
            )
        })?;
    if registry.schema != "python-slm-sensitive-rules-v1"
        || registry.policy_id != SENSITIVE_POLICY_ID
        || registry.registry_id != SENSITIVE_REGISTRY_ID
        || registry.normalization != "utf8-bytes-ascii-casefold-keywords-v1"
        || registry.entropy.minimum_length != 20
        || registry.entropy.maximum_length != 256
        || registry.entropy.minimum_millibits_per_byte != 3_500
        || registry.entropy.minimum_character_classes != 3
        || sha256(REGISTRY_BYTES) != REGISTRY_SHA256
        || !strictly_sorted_unique(&registry.reserved_email_domains)
        || !strictly_sorted_unique(&registry.secret_like_names)
        || !strictly_sorted_unique(&registry.confirmed_rule_ids)
        || !strictly_sorted_unique(&registry.quarantine_rule_ids)
    {
        return Err(ProductError::integrity(
            "SENSITIVE_REGISTRY_INVALID",
            "the embedded sensitive-data registry violates its frozen contract",
        ));
    }
    Ok(registry)
}

pub fn policy_binding() -> Result<SensitivePolicyBinding> {
    let _ = policy_registry()?;
    Ok(SensitivePolicyBinding {
        schema: SENSITIVE_BINDING_SCHEMA,
        policy_id: SENSITIVE_POLICY_ID,
        registry_id: SENSITIVE_REGISTRY_ID,
        registry_sha256: REGISTRY_SHA256.to_owned(),
    })
}

pub fn evaluate_sensitive_policy(
    source: &[u8],
    cancellation: &CancellationToken,
) -> Result<SensitivePolicyResult> {
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    let registry = policy_registry()?;
    let mut confirmed = BTreeSet::new();
    let mut uncertain = BTreeSet::new();

    for marker in PRIVATE_KEY_MARKERS {
        if contains_bytes(source, marker) {
            confirmed.insert("PRIVATE_KEY_PEM");
            break;
        }
    }
    scan_provider_credentials(source, &mut confirmed, cancellation)?;
    if contains_credentialed_url(source, cancellation)? {
        confirmed.insert("CREDENTIALED_URL");
    }
    scan_lines(
        source,
        &registry,
        &mut confirmed,
        &mut uncertain,
        cancellation,
    )?;
    scan_emails(source, &registry, &mut confirmed, cancellation)?;
    scan_telephone_numbers(source, &mut confirmed, cancellation)?;
    scan_government_identifiers(source, &mut confirmed, cancellation)?;
    scan_payment_accounts(source, &mut confirmed, cancellation)?;

    let (status, reason, rules) = if !confirmed.is_empty() {
        ("REJECTED", Some("SENSITIVE_CONTENT_DETECTED"), confirmed)
    } else if !uncertain.is_empty() {
        (
            "QUARANTINED",
            Some("SENSITIVE_CONTENT_UNCERTAIN"),
            uncertain,
        )
    } else {
        ("ACCEPTED", None, BTreeSet::new())
    };
    Ok(SensitivePolicyResult {
        schema: SENSITIVE_RESULT_SCHEMA,
        policy: policy_binding()?,
        source_sha256: sha256(source),
        source_bytes: source.len() as u64,
        status,
        reason,
        finding_count: rules.len() as u64,
        rule_ids: rules.into_iter().map(str::to_owned).collect(),
        restricted_values_emitted: false,
        source_rewritten: false,
    })
}

fn scan_provider_credentials(
    source: &[u8],
    confirmed: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    for index in 0..source.len() {
        if index % 1_024 == 0 && cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if index > 0 && token_char(&source[index - 1]) {
            continue;
        }
        let tail = &source[index..];
        let matched = has_fixed_token(tail, b"AKIA", 16, upper_alnum)
            || has_fixed_token(tail, b"ASIA", 16, upper_alnum)
            || has_fixed_token(tail, b"ghp_", 36, token_char)
            || has_fixed_token(tail, b"gho_", 36, token_char)
            || has_fixed_token(tail, b"ghu_", 36, token_char)
            || has_fixed_token(tail, b"ghs_", 36, token_char)
            || has_fixed_token(tail, b"ghr_", 36, token_char)
            || has_min_token(tail, b"github_pat_", 22, token_char)
            || has_fixed_token(tail, b"AIza", 35, token_char)
            || has_min_token(tail, b"sk_live_", 16, token_char)
            || slack_token(tail);
        if matched {
            confirmed.insert("PROVIDER_CREDENTIAL");
            return Ok(());
        }
    }
    Ok(())
}

fn scan_lines(
    source: &[u8],
    registry: &SensitivePolicyRegistry,
    confirmed: &mut BTreeSet<&'static str>,
    uncertain: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    for line in source.split(|byte| *byte == b'\n') {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if let Some((name, value)) = assigned_string(line) {
            let name = String::from_utf8_lossy(name).to_ascii_lowercase();
            if secret_like_name(&name, &registry.secret_like_names) {
                if entropy_qualifies(&value, &registry.entropy) {
                    confirmed.insert("HIGH_ENTROPY_NAMED_SECRET");
                } else if value.len() >= 8 && !obvious_placeholder(&value) {
                    uncertain.insert("POSSIBLE_NAMED_SECRET");
                }
            }
            if looks_like_postal_address(&value) {
                if contains_postal_code(&value) {
                    confirmed.insert("POSTAL_ADDRESS");
                } else {
                    uncertain.insert("POSSIBLE_POSTAL_ADDRESS");
                }
            }
            if government_like_name(&name)
                && value.iter().filter(|byte| byte.is_ascii_digit()).count() >= 8
                && !contains_valid_ssn(&value)
            {
                uncertain.insert("POSSIBLE_GOVERNMENT_IDENTIFIER");
            }
            if telephone_like_name(&name) && telephone_digit_count(&value).is_some() {
                confirmed.insert("TELEPHONE_NUMBER");
            }
        }
    }
    Ok(())
}

fn scan_emails(
    source: &[u8],
    registry: &SensitivePolicyRegistry,
    confirmed: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    for at in source
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'@').then_some(index))
    {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let start = source[..at]
            .iter()
            .rposition(|byte| !email_local(*byte))
            .map_or(0, |index| index + 1);
        let end = source[at + 1..]
            .iter()
            .position(|byte| !email_domain(*byte))
            .map_or(source.len(), |index| at + 1 + index);
        let local = &source[start..at];
        let domain = &source[at + 1..end];
        if valid_email(local, domain) && !reserved_domain(domain, &registry.reserved_email_domains)
        {
            confirmed.insert("PERSONAL_EMAIL");
            return Ok(());
        }
    }
    Ok(())
}

fn scan_telephone_numbers(
    source: &[u8],
    confirmed: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    for index in 0..source.len() {
        if index % 1_024 == 0 && cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if source[index] != b'+' || index > 0 && source[index - 1].is_ascii_alphanumeric() {
            continue;
        }
        let mut cursor = index + 1;
        let mut digits = 0;
        while cursor < source.len()
            && (source[cursor].is_ascii_digit()
                || matches!(source[cursor], b' ' | b'-' | b'(' | b')'))
        {
            digits += usize::from(source[cursor].is_ascii_digit());
            cursor += 1;
        }
        if (10..=15).contains(&digits)
            && (cursor == source.len() || !source[cursor].is_ascii_digit())
        {
            confirmed.insert("TELEPHONE_NUMBER");
            return Ok(());
        }
    }
    Ok(())
}

fn scan_government_identifiers(
    source: &[u8],
    confirmed: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    for window_start in 0..source.len().saturating_sub(10) {
        if window_start % 1_024 == 0 && cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if (window_start == 0 || !source[window_start - 1].is_ascii_digit())
            && (window_start + 11 == source.len() || !source[window_start + 11].is_ascii_digit())
            && contains_valid_ssn(&source[window_start..window_start + 11])
        {
            confirmed.insert("GOVERNMENT_IDENTIFIER");
            return Ok(());
        }
    }
    Ok(())
}

fn scan_payment_accounts(
    source: &[u8],
    confirmed: &mut BTreeSet<&'static str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    if contains_valid_iban(source, cancellation)? {
        confirmed.insert("PAYMENT_ACCOUNT_IDENTIFIER");
        return Ok(());
    }
    let mut index = 0;
    while index < source.len() {
        if index % 1_024 == 0 && cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if source[index].is_ascii_digit()
            && (index == 0 || !source[index - 1].is_ascii_alphanumeric())
        {
            let mut cursor = index;
            let mut digits = Vec::new();
            while cursor < source.len()
                && (source[cursor].is_ascii_digit() || matches!(source[cursor], b' ' | b'-'))
            {
                if source[cursor].is_ascii_digit() {
                    digits.push(source[cursor] - b'0');
                }
                cursor += 1;
            }
            if (13..=19).contains(&digits.len())
                && (cursor == source.len() || !source[cursor].is_ascii_alphanumeric())
                && !all_equal(&digits)
                && luhn_valid(&digits)
                && !reserved_test_card(&digits)
            {
                confirmed.insert("PAYMENT_ACCOUNT_IDENTIFIER");
                return Ok(());
            }
            index = cursor.max(index + 1);
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn contains_credentialed_url(source: &[u8], cancellation: &CancellationToken) -> Result<bool> {
    for scheme in find_all(source, b"://") {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let authority = &source[scheme + 3..];
        let end = authority
            .iter()
            .position(|byte| {
                matches!(
                    byte,
                    b'/' | b'\\' | b' ' | b'\t' | b'\r' | b'\n' | b'\'' | b'"'
                )
            })
            .unwrap_or(authority.len());
        let authority = &authority[..end];
        if let Some(at) = authority.iter().position(|byte| *byte == b'@')
            && authority[..at]
                .iter()
                .position(|byte| *byte == b':')
                .is_some_and(|colon| colon > 0 && colon + 1 < at)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn assigned_string(line: &[u8]) -> Option<(&[u8], Vec<u8>)> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    let left = trim_ascii(&line[..equals]);
    let left = left
        .iter()
        .position(|byte| *byte == b':')
        .map_or(left, |colon| trim_ascii(&left[..colon]));
    let name_start = left
        .iter()
        .rposition(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .map_or(0, |index| index + 1);
    let name = &left[name_start..];
    if name.is_empty() || !name[0].is_ascii_alphabetic() && name[0] != b'_' {
        return None;
    }
    let mut right = trim_ascii(&line[equals + 1..]);
    let mut value = Vec::new();
    loop {
        let (fragment, consumed) = python_string_fragment(right)?;
        value.extend_from_slice(fragment);
        right = trim_ascii(&right[consumed..]);
        if right.is_empty() || right.starts_with(b"#") {
            break;
        }
        if !starts_python_string(right) {
            break;
        }
    }
    Some((name, value))
}

fn starts_python_string(value: &[u8]) -> bool {
    python_string_fragment(value).is_some()
}

fn python_string_fragment(value: &[u8]) -> Option<(&[u8], usize)> {
    let mut quote_at = 0;
    while quote_at < value.len()
        && quote_at < 2
        && matches!(
            value[quote_at],
            b'r' | b'R' | b'b' | b'B' | b'u' | b'U' | b'f' | b'F'
        )
    {
        quote_at += 1;
    }
    let quote = *value.get(quote_at)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let triple = value.get(quote_at..quote_at + 3) == Some(&[quote, quote, quote]);
    let delimiter = if triple { 3 } else { 1 };
    let start = quote_at + delimiter;
    let mut at = start;
    while at < value.len() {
        if triple {
            if value.get(at..at + 3) == Some(&[quote, quote, quote]) {
                return Some((&value[start..at], at + 3));
            }
        } else if value[at] == quote {
            return Some((&value[start..at], at + 1));
        }
        if value[at] == b'\\' && at + 1 < value.len() {
            at += 2;
        } else {
            at += 1;
        }
    }
    None
}
fn entropy_qualifies(value: &[u8], policy: &EntropyPolicy) -> bool {
    if !(policy.minimum_length..=policy.maximum_length).contains(&value.len())
        || character_classes(value) < policy.minimum_character_classes
    {
        return false;
    }
    let mut counts = [0_u64; 256];
    for byte in value {
        counts[*byte as usize] += 1;
    }
    let length = value.len() as f64;
    let bits = counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let probability = *count as f64 / length;
            -probability * probability.log2()
        })
        .sum::<f64>();
    bits * 1_000.0 >= policy.minimum_millibits_per_byte as f64
}

fn character_classes(value: &[u8]) -> u64 {
    [
        value.iter().any(u8::is_ascii_lowercase),
        value.iter().any(u8::is_ascii_uppercase),
        value.iter().any(u8::is_ascii_digit),
        value.iter().any(|byte| !byte.is_ascii_alphanumeric()),
    ]
    .into_iter()
    .map(u64::from)
    .sum()
}

fn secret_like_name(name: &str, names: &[String]) -> bool {
    names.iter().any(|candidate| {
        name == candidate
            || name
                .strip_suffix(candidate)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('_'))
    })
}

fn government_like_name(name: &str) -> bool {
    ["government_id", "national_id", "passport", "ssn", "tax_id"]
        .iter()
        .any(|candidate| name == *candidate || name.ends_with(&format!("_{candidate}")))
}

fn telephone_like_name(name: &str) -> bool {
    ["mobile", "phone", "telephone"]
        .iter()
        .any(|candidate| name == *candidate || name.ends_with(&format!("_{candidate}")))
}

fn telephone_digit_count(value: &[u8]) -> Option<usize> {
    if value
        .iter()
        .any(|byte| !byte.is_ascii_digit() && !matches!(byte, b' ' | b'-' | b'(' | b')' | b'+'))
    {
        return None;
    }
    let digits = value.iter().filter(|byte| byte.is_ascii_digit()).count();
    (10..=15).contains(&digits).then_some(digits)
}

fn obvious_placeholder(value: &[u8]) -> bool {
    let lower = value.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    [
        b"changeme".as_slice(),
        b"example".as_slice(),
        b"not-a-secret".as_slice(),
        b"placeholder".as_slice(),
    ]
    .contains(&lower.as_slice())
}

fn looks_like_postal_address(value: &[u8]) -> bool {
    let text = String::from_utf8_lossy(value).to_ascii_lowercase();
    let starts_with_number = text
        .split_ascii_whitespace()
        .next()
        .is_some_and(|part| part.chars().all(|character| character.is_ascii_digit()));
    starts_with_number
        && STREET_SUFFIXES.iter().any(|suffix| {
            text.split(|character: char| !character.is_ascii_alphanumeric())
                .any(|word| word == *suffix)
        })
}

fn contains_postal_code(value: &[u8]) -> bool {
    value
        .split(|byte| !byte.is_ascii_digit())
        .any(|digits| matches!(digits.len(), 5 | 6) && digits.iter().all(u8::is_ascii_digit))
}

fn contains_valid_ssn(value: &[u8]) -> bool {
    value.windows(11).any(|candidate| {
        candidate[0..3].iter().all(u8::is_ascii_digit)
            && candidate[3] == b'-'
            && candidate[4..6].iter().all(u8::is_ascii_digit)
            && candidate[6] == b'-'
            && candidate[7..11].iter().all(u8::is_ascii_digit)
            && candidate[0..3] != *b"000"
            && candidate[0..3] != *b"666"
            && candidate[0] != b'9'
            && candidate[4..6] != *b"00"
            && candidate[7..11] != *b"0000"
    })
}

fn contains_valid_iban(source: &[u8], cancellation: &CancellationToken) -> Result<bool> {
    for (index, token) in source
        .split(|byte| !byte.is_ascii_alphanumeric())
        .enumerate()
    {
        if index % 256 == 0 && cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if valid_iban(token) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn valid_iban(value: &[u8]) -> bool {
    if !(15..=34).contains(&value.len())
        || !value[..2].iter().all(u8::is_ascii_alphabetic)
        || !value[2..4].iter().all(u8::is_ascii_digit)
        || !value[4..].iter().all(u8::is_ascii_alphanumeric)
    {
        return false;
    }
    let mut remainder = 0_u32;
    for byte in value[4..].iter().chain(&value[..4]) {
        let upper = byte.to_ascii_uppercase();
        if upper.is_ascii_digit() {
            remainder = (remainder * 10 + u32::from(upper - b'0')) % 97;
        } else if upper.is_ascii_uppercase() {
            let expanded = u32::from(upper - b'A') + 10;
            remainder = (remainder * 100 + expanded) % 97;
        } else {
            return false;
        }
    }
    remainder == 1
}

fn luhn_valid(digits: &[u8]) -> bool {
    let parity = digits.len() % 2;
    digits
        .iter()
        .enumerate()
        .map(|(index, digit)| {
            if index % 2 == parity {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                *digit
            }
        })
        .map(u64::from)
        .sum::<u64>()
        % 10
        == 0
}

fn reserved_test_card(digits: &[u8]) -> bool {
    [
        b"4111111111111111".as_slice(),
        b"4242424242424242".as_slice(),
        b"5555555555554444".as_slice(),
    ]
    .iter()
    .any(|reserved| {
        reserved
            .iter()
            .map(|digit| digit - b'0')
            .eq(digits.iter().copied())
    })
}

fn valid_email(local: &[u8], domain: &[u8]) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with(b".")
        && !local.ends_with(b".")
        && !local.windows(2).any(|window| window == b"..")
        && domain.len() <= 253
        && domain.contains(&b'.')
        && !domain.starts_with(b".")
        && !domain.ends_with(b".")
        && !domain.windows(2).any(|window| window == b"..")
}

fn reserved_domain(domain: &[u8], reserved: &[String]) -> bool {
    let domain = String::from_utf8_lossy(domain).to_ascii_lowercase();
    reserved
        .iter()
        .any(|candidate| domain == *candidate || domain.ends_with(&format!(".{candidate}")))
}

fn find_all<'a>(source: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    source
        .windows(needle.len())
        .enumerate()
        .filter_map(move |(index, window)| (window == needle).then_some(index))
}

fn has_fixed_token(
    tail: &[u8],
    prefix: &[u8],
    suffix_length: usize,
    predicate: fn(&u8) -> bool,
) -> bool {
    tail.starts_with(prefix)
        && tail
            .get(prefix.len()..prefix.len() + suffix_length)
            .is_some_and(|suffix| suffix.iter().all(predicate))
        && tail
            .get(prefix.len() + suffix_length)
            .is_none_or(|byte| !predicate(byte))
}

fn has_min_token(
    tail: &[u8],
    prefix: &[u8],
    minimum_suffix: usize,
    predicate: fn(&u8) -> bool,
) -> bool {
    if !tail.starts_with(prefix) {
        return false;
    }
    tail[prefix.len()..]
        .iter()
        .take_while(|byte| predicate(byte))
        .count()
        >= minimum_suffix
}

fn slack_token(tail: &[u8]) -> bool {
    tail.starts_with(b"xox")
        && tail.get(3).is_some_and(|byte| b"baprs".contains(byte))
        && tail.get(4) == Some(&b'-')
        && tail[5..].iter().take_while(|byte| token_char(byte)).count() >= 10
}

fn strictly_sorted_unique(values: &[String]) -> bool {
    !values.is_empty() && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn contains_bytes(source: &[u8], needle: &[u8]) -> bool {
    source.windows(needle.len()).any(|window| window == needle)
}

fn upper_alnum(byte: &u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit()
}

fn token_char(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn email_local(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte)
}

fn email_domain(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
}

fn all_equal(values: &[u8]) -> bool {
    values
        .first()
        .is_none_or(|first| values.iter().all(|value| value == first))
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn cancelled() -> ProductError {
    ProductError::gate(
        "SENSITIVE_POLICY_CANCELLED",
        "the sensitive-data policy operation was cancelled",
    )
}
