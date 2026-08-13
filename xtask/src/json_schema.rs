use crate::error::{Result, XtaskError};
use regex::Regex;
use serde_json::{Map, Value};

const MAX_SCHEMA_DEPTH: usize = 256;

pub(crate) fn validate(schema: &Value, instance: &Value, code: &'static str) -> Result<()> {
    let validator = Validator { root: schema, code };
    validator.validate_schema_shape(schema, "$schema", 0)?;
    validator.validate_at(schema, instance, "$", 0)
}

pub(crate) fn validate_schema_document(schema: &Value, code: &'static str) -> Result<()> {
    Validator { root: schema, code }.validate_schema_shape(schema, "$schema", 0)
}

struct Validator<'a> {
    root: &'a Value,
    code: &'static str,
}

impl Validator<'_> {
    fn validate_schema_shape(&self, schema: &Value, path: &str, depth: usize) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(XtaskError::integrity(
                self.code,
                format!("schema definition recursion exceeded at {path}"),
            ));
        }
        if schema.is_boolean() {
            return Ok(());
        }
        let object = schema.as_object().ok_or_else(|| {
            XtaskError::integrity(self.code, format!("schema at {path} is not an object"))
        })?;
        if let Some(reference) = object.get("$ref") {
            let reference = reference.as_str().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("$ref at {path} is not a string"))
            })?;
            let _ = self.resolve_reference(reference, path)?;
        }
        if let Some(kind) = object.get("type") {
            let kinds = if let Some(kind) = kind.as_str() {
                vec![kind]
            } else {
                kind.as_array()
                    .ok_or_else(|| {
                        XtaskError::integrity(self.code, format!("type at {path} is invalid"))
                    })?
                    .iter()
                    .map(|item| {
                        item.as_str().ok_or_else(|| {
                            XtaskError::integrity(
                                self.code,
                                format!("type at {path} contains a non-string"),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            if kinds.is_empty()
                || kinds.iter().any(|kind| {
                    !matches!(
                        *kind,
                        "null" | "boolean" | "object" | "array" | "number" | "integer" | "string"
                    )
                })
            {
                return Err(XtaskError::integrity(
                    self.code,
                    format!("type at {path} is outside the supported Draft-07 set"),
                ));
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(value) = object.get(keyword) {
                for (index, child) in schema_array(value, keyword, path, self.code)?
                    .iter()
                    .enumerate()
                {
                    self.validate_schema_shape(
                        child,
                        &format!("{path}.{keyword}[{index}]"),
                        depth + 1,
                    )?;
                }
            }
        }
        for keyword in ["not", "if", "then", "else", "contains"] {
            if let Some(child) = object.get(keyword) {
                self.validate_schema_shape(child, &format!("{path}.{keyword}"), depth + 1)?;
            }
        }
        if let Some(items) = object.get("items") {
            if let Some(items) = items.as_array() {
                for (index, child) in items.iter().enumerate() {
                    self.validate_schema_shape(
                        child,
                        &format!("{path}.items[{index}]"),
                        depth + 1,
                    )?;
                }
            } else {
                self.validate_schema_shape(items, &format!("{path}.items"), depth + 1)?;
            }
        }
        for keyword in ["properties", "definitions"] {
            if let Some(children) = object.get(keyword) {
                let children = children.as_object().ok_or_else(|| {
                    XtaskError::integrity(
                        self.code,
                        format!("{keyword} at {path} is not an object"),
                    )
                })?;
                for (name, child) in children {
                    self.validate_schema_shape(
                        child,
                        &format!("{path}.{keyword}.{name}"),
                        depth + 1,
                    )?;
                }
            }
        }
        if let Some(additional) = object.get("additionalProperties")
            && !additional.is_boolean()
        {
            self.validate_schema_shape(
                additional,
                &format!("{path}.additionalProperties"),
                depth + 1,
            )?;
        }
        if let Some(required) = object.get("required") {
            let _ = string_array(required, "required", path, self.code)?;
        }
        if let Some(values) = object.get("enum")
            && !values.is_array()
        {
            return Err(XtaskError::integrity(
                self.code,
                format!("enum at {path} is not an array"),
            ));
        }
        for keyword in [
            "minItems",
            "maxItems",
            "minProperties",
            "maxProperties",
            "minLength",
            "maxLength",
        ] {
            if let Some(value) = object.get(keyword)
                && value.as_u64().is_none()
            {
                return Err(XtaskError::integrity(
                    self.code,
                    format!("{keyword} at {path} is not a nonnegative integer"),
                ));
            }
        }
        for keyword in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
            if let Some(value) = object.get(keyword)
                && value.as_f64().is_none()
            {
                return Err(XtaskError::integrity(
                    self.code,
                    format!("{keyword} at {path} is not numeric"),
                ));
            }
        }
        if let Some(pattern) = object.get("pattern") {
            let pattern = pattern.as_str().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("pattern at {path} is not a string"))
            })?;
            let _ = matches_pattern(pattern, "")?;
        }
        if let Some(format) = object.get("format")
            && format.as_str() != Some("date-time")
        {
            return Err(XtaskError::integrity(
                self.code,
                format!("unsupported format at {path}"),
            ));
        }
        Ok(())
    }

    fn validate_at(
        &self,
        schema: &Value,
        instance: &Value,
        path: &str,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return self.failure(path, "schema recursion exceeded the closed depth limit");
        }
        if let Some(allowed) = schema.as_bool() {
            return if allowed {
                Ok(())
            } else {
                self.failure(path, "instance is rejected by a false schema")
            };
        }
        let object = schema.as_object().ok_or_else(|| {
            XtaskError::integrity(self.code, format!("schema at {path} is not an object"))
        })?;

        if let Some(reference) = object.get("$ref") {
            let reference = reference.as_str().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("$ref at {path} is not a string"))
            })?;
            let target = self.resolve_reference(reference, path)?;
            self.validate_at(target, instance, path, depth + 1)?;
        }
        if let Some(expected) = object.get("const")
            && instance != expected
        {
            return self.failure(path, "value differs from the schema constant");
        }
        if let Some(values) = object.get("enum") {
            let values = values.as_array().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("enum at {path} is not an array"))
            })?;
            if !values.iter().any(|candidate| candidate == instance) {
                return self.failure(path, "value is outside the closed schema enumeration");
            }
        }
        if let Some(kind) = object.get("type")
            && !matches_type(kind, instance)
        {
            return self.failure(path, "value has the wrong JSON type");
        }

        self.validate_combinators(object, instance, path, depth)?;
        if let Some(instance_object) = instance.as_object() {
            self.validate_object(object, instance_object, path, depth)?;
        }
        if let Some(instance_array) = instance.as_array() {
            self.validate_array(object, instance_array, path, depth)?;
        }
        if let Some(text) = instance.as_str() {
            self.validate_string(object, text, path)?;
        }
        if instance.is_number() {
            self.validate_number(object, instance, path)?;
        }
        Ok(())
    }

    fn validate_combinators(
        &self,
        schema: &Map<String, Value>,
        instance: &Value,
        path: &str,
        depth: usize,
    ) -> Result<()> {
        if let Some(all) = schema.get("allOf") {
            for member in schema_array(all, "allOf", path, self.code)? {
                self.validate_at(member, instance, path, depth + 1)?;
            }
        }
        if let Some(any) = schema.get("anyOf") {
            let members = schema_array(any, "anyOf", path, self.code)?;
            if !members
                .iter()
                .any(|member| self.validate_at(member, instance, path, depth + 1).is_ok())
            {
                return self.failure(path, "value matches no anyOf branch");
            }
        }
        if let Some(one) = schema.get("oneOf") {
            let members = schema_array(one, "oneOf", path, self.code)?;
            let matches = members
                .iter()
                .filter(|member| self.validate_at(member, instance, path, depth + 1).is_ok())
                .count();
            if matches != 1 {
                return self.failure(path, "value does not match exactly one oneOf branch");
            }
        }
        if let Some(negated) = schema.get("not")
            && self.validate_at(negated, instance, path, depth + 1).is_ok()
        {
            return self.failure(path, "value matches a forbidden schema branch");
        }
        if let Some(condition) = schema.get("if") {
            let selected = if self
                .validate_at(condition, instance, path, depth + 1)
                .is_ok()
            {
                schema.get("then")
            } else {
                schema.get("else")
            };
            if let Some(selected) = selected {
                self.validate_at(selected, instance, path, depth + 1)?;
            }
        }
        Ok(())
    }

    fn validate_object(
        &self,
        schema: &Map<String, Value>,
        instance: &Map<String, Value>,
        path: &str,
        depth: usize,
    ) -> Result<()> {
        if let Some(required) = schema.get("required") {
            for key in string_array(required, "required", path, self.code)? {
                if !instance.contains_key(key) {
                    return self.failure(&format!("{path}.{key}"), "required property is missing");
                }
            }
        }
        let properties = match schema.get("properties") {
            Some(value) => Some(value.as_object().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("properties at {path} is not an object"))
            })?),
            None => None,
        };
        for (key, value) in instance {
            if let Some(property_schema) = properties.and_then(|values| values.get(key)) {
                self.validate_at(property_schema, value, &format!("{path}.{key}"), depth + 1)?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return self.failure(&format!("{path}.{key}"), "additional property is forbidden");
            } else if let Some(additional) = schema
                .get("additionalProperties")
                .filter(|value| !value.is_boolean())
            {
                self.validate_at(additional, value, &format!("{path}.{key}"), depth + 1)?;
            }
        }
        compare_usize_bound(
            schema.get("minProperties"),
            instance.len(),
            true,
            path,
            self.code,
        )?;
        compare_usize_bound(
            schema.get("maxProperties"),
            instance.len(),
            false,
            path,
            self.code,
        )
    }

    fn validate_array(
        &self,
        schema: &Map<String, Value>,
        instance: &[Value],
        path: &str,
        depth: usize,
    ) -> Result<()> {
        compare_usize_bound(
            schema.get("minItems"),
            instance.len(),
            true,
            path,
            self.code,
        )?;
        compare_usize_bound(
            schema.get("maxItems"),
            instance.len(),
            false,
            path,
            self.code,
        )?;
        if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
            for (index, item) in instance.iter().enumerate() {
                if instance[..index].iter().any(|prior| prior == item) {
                    return self.failure(path, "array contains a duplicate item");
                }
            }
        }
        if let Some(items) = schema.get("items") {
            if let Some(tuple) = items.as_array() {
                for (index, item_schema) in tuple.iter().enumerate() {
                    if let Some(item) = instance.get(index) {
                        self.validate_at(
                            item_schema,
                            item,
                            &format!("{path}[{index}]"),
                            depth + 1,
                        )?;
                    }
                }
            } else {
                for (index, item) in instance.iter().enumerate() {
                    self.validate_at(items, item, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
        }
        if let Some(contains) = schema.get("contains")
            && !instance.iter().enumerate().any(|(index, item)| {
                self.validate_at(contains, item, &format!("{path}[{index}]"), depth + 1)
                    .is_ok()
            })
        {
            return self.failure(path, "array has no item matching contains");
        }
        Ok(())
    }

    fn validate_string(
        &self,
        schema: &Map<String, Value>,
        instance: &str,
        path: &str,
    ) -> Result<()> {
        let length = instance.chars().count();
        compare_usize_bound(schema.get("minLength"), length, true, path, self.code)?;
        compare_usize_bound(schema.get("maxLength"), length, false, path, self.code)?;
        if let Some(pattern) = schema.get("pattern") {
            let pattern = pattern.as_str().ok_or_else(|| {
                XtaskError::integrity(self.code, format!("pattern at {path} is not a string"))
            })?;
            if !matches_pattern(pattern, instance)? {
                return self.failure(path, "string does not match its closed schema pattern");
            }
        }
        if schema.get("format").and_then(Value::as_str) == Some("date-time")
            && !valid_utc_date_time(instance)
        {
            return self.failure(path, "date-time is not a real UTC calendar timestamp");
        }
        Ok(())
    }

    fn validate_number(
        &self,
        schema: &Map<String, Value>,
        instance: &Value,
        path: &str,
    ) -> Result<()> {
        let actual = instance.as_f64().ok_or_else(|| {
            XtaskError::integrity(self.code, format!("number at {path} is not finite"))
        })?;
        for (keyword, inclusive, lower) in [
            ("minimum", true, true),
            ("maximum", true, false),
            ("exclusiveMinimum", false, true),
            ("exclusiveMaximum", false, false),
        ] {
            if let Some(bound) = schema.get(keyword) {
                let bound = bound.as_f64().ok_or_else(|| {
                    XtaskError::integrity(
                        self.code,
                        format!("numeric bound {keyword} at {path} is invalid"),
                    )
                })?;
                let valid = match (lower, inclusive) {
                    (true, true) => actual >= bound,
                    (true, false) => actual > bound,
                    (false, true) => actual <= bound,
                    (false, false) => actual < bound,
                };
                if !valid {
                    return self.failure(path, "number is outside its schema bound");
                }
            }
        }
        Ok(())
    }

    fn resolve_reference<'a>(&'a self, reference: &str, path: &str) -> Result<&'a Value> {
        let pointer = reference.strip_prefix('#').ok_or_else(|| {
            XtaskError::integrity(
                self.code,
                format!("external schema reference {reference:?} is forbidden at {path}"),
            )
        })?;
        self.root.pointer(pointer).ok_or_else(|| {
            XtaskError::integrity(
                self.code,
                format!("schema reference {reference:?} is unresolved at {path}"),
            )
        })
    }

    fn failure<T>(&self, path: &str, reason: &str) -> Result<T> {
        Err(XtaskError::integrity(
            self.code,
            format!("schema validation failed at {path}: {reason}"),
        ))
    }
}

fn matches_type(schema: &Value, instance: &Value) -> bool {
    let matches = |kind: &str| match kind {
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "string" => instance.is_string(),
        _ => false,
    };
    schema.as_str().is_some_and(&matches)
        || schema
            .as_array()
            .is_some_and(|values| values.iter().filter_map(Value::as_str).any(matches))
}

fn schema_array<'a>(
    value: &'a Value,
    keyword: &str,
    path: &str,
    code: &'static str,
) -> Result<&'a [Value]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| XtaskError::integrity(code, format!("{keyword} at {path} is not an array")))
}

fn string_array<'a>(
    value: &'a Value,
    keyword: &str,
    path: &str,
    code: &'static str,
) -> Result<Vec<&'a str>> {
    schema_array(value, keyword, path, code)?
        .iter()
        .map(|item| {
            item.as_str().ok_or_else(|| {
                XtaskError::integrity(code, format!("{keyword} at {path} contains a non-string"))
            })
        })
        .collect()
}

fn compare_usize_bound(
    bound: Option<&Value>,
    actual: usize,
    minimum: bool,
    path: &str,
    code: &'static str,
) -> Result<()> {
    let Some(bound) = bound else {
        return Ok(());
    };
    let bound = bound.as_u64().ok_or_else(|| {
        XtaskError::integrity(code, format!("collection bound at {path} is invalid"))
    })?;
    let valid = if minimum {
        actual as u64 >= bound
    } else {
        actual as u64 <= bound
    };
    if valid {
        Ok(())
    } else {
        Err(XtaskError::integrity(
            code,
            format!("collection length at {path} violates its schema bound"),
        ))
    }
}

fn matches_pattern(pattern: &str, instance: &str) -> Result<bool> {
    if pattern.contains("(?!") {
        return Ok(match pattern {
            value if value.contains("RUSTUP_HOME|CARGO_HOME|VS_INSTALL") => {
                valid_tokenized_path(instance)
            }
            value if value.contains("[A-Za-z0-9_$(){}+-]") => valid_relative_path(instance, true),
            value if value.contains("[A-Za-z0-9_-][A-Za-z0-9._-]") => {
                valid_relative_path(instance, false)
            }
            _ => {
                return Err(XtaskError::integrity(
                    "JSON_SCHEMA_PATTERN_UNSUPPORTED",
                    format!("unsupported look-around schema pattern: {pattern}"),
                ));
            }
        });
    }
    Regex::new(pattern)
        .map(|compiled| compiled.is_match(instance))
        .map_err(|error| {
            XtaskError::integrity(
                "JSON_SCHEMA_PATTERN_INVALID",
                format!("invalid schema pattern {pattern:?}: {error}"),
            )
        })
}

fn valid_relative_path(value: &str, receipt_characters: bool) -> bool {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value
            .as_bytes()
            .get(1)
            .is_some_and(|character| *character == b':')
    {
        return false;
    }
    value.split('/').all(|component| {
        !component.is_empty()
            && !matches!(component, "." | "..")
            && component.chars().enumerate().all(|(index, character)| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '_' | '-')
                    || (index > 0 && character == '.')
                    || (receipt_characters
                        && matches!(character, '$' | '(' | ')' | '{' | '}' | '+'))
            })
    })
}

fn valid_tokenized_path(value: &str) -> bool {
    const ROOTS: [&str; 8] = [
        "RUSTUP_HOME",
        "CARGO_HOME",
        "VS_INSTALL",
        "VC_TOOLS",
        "WINDOWS_KITS",
        "SYSTEM32",
        "PROGRAM_FILES",
        "PROGRAM_FILES_X86",
    ];
    let Some(rest) = value.strip_prefix("${") else {
        return false;
    };
    let Some((root, suffix)) = rest.split_once('}') else {
        return false;
    };
    if !ROOTS.contains(&root) || (!suffix.is_empty() && !suffix.starts_with('/')) {
        return false;
    }
    if suffix.is_empty() {
        return true;
    }
    let Some(components) = suffix.strip_prefix('/') else {
        return false;
    };
    !components.is_empty()
        && components.split('/').all(|component| {
            !component.is_empty()
                && !matches!(component, "." | "..")
                && component.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, ' ' | '.' | '_' | '(' | ')' | '+' | '-')
                })
        })
}

fn valid_utc_date_time(value: &str) -> bool {
    let Some(prefix) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = prefix.split_once('T') else {
        return false;
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    let (clock, fraction) = time
        .split_once('.')
        .map_or((time, None), |(clock, fraction)| (clock, Some(fraction)));
    if fraction.is_some_and(|value| {
        value.is_empty() || !value.bytes().all(|character| character.is_ascii_digit())
    }) {
        return false;
    }
    let mut time_parts = clock.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || hour.len() != 2
        || minute.len() != 2
        || second.len() != 2
    {
        return false;
    }
    let Ok(year) = year.parse::<u32>() else {
        return false;
    };
    let (Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second)) = (
        month.parse::<u32>(),
        day.parse::<u32>(),
        hour.parse::<u32>(),
        minute.parse::<u32>(),
        second.parse::<u32>(),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let maximum_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=maximum_day).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn closed_objects_and_conditionals_are_enforced() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["status", "value"],
            "properties": {
                "status": {"enum": ["PASS", "FAIL"]},
                "value": {"type": "integer"}
            },
            "if": {"properties": {"status": {"const": "PASS"}}},
            "then": {"properties": {"value": {"const": 7}}},
            "else": {"properties": {"value": {"const": 0}}}
        });
        assert!(validate(&schema, &json!({"status": "PASS", "value": 7}), "TEST").is_ok());
        assert!(validate(&schema, &json!({"status": "PASS", "value": 0}), "TEST").is_err());
        assert!(
            validate(
                &schema,
                &json!({"status": "FAIL", "value": 0, "extra": 1}),
                "TEST"
            )
            .is_err()
        );
    }

    #[test]
    fn local_refs_arrays_and_one_of_are_enforced() {
        let schema = json!({
            "oneOf": [
                {"type": "null"},
                {"type": "array", "minItems": 2, "uniqueItems": true,
                 "contains": {"const": "required"}, "items": {"$ref": "#/definitions/item"}}
            ],
            "definitions": {"item": {"type": "string", "minLength": 1}}
        });
        assert!(validate(&schema, &json!(["required", "other"]), "TEST").is_ok());
        assert!(validate(&schema, &json!(["required", "required"]), "TEST").is_err());
        assert!(validate(&schema, &json!(["other", "second"]), "TEST").is_err());
    }

    #[test]
    fn portable_paths_and_calendar_times_are_strict() {
        assert!(valid_relative_path("commands/C001.stdout.txt", true));
        assert!(!valid_relative_path("../escape", true));
        assert!(valid_tokenized_path("${PROGRAM_FILES}/Git/cmd/git.exe"));
        assert!(!valid_tokenized_path("${PROGRAM_FILES}/Git/../escape"));
        assert!(valid_utc_date_time("2024-02-29T23:59:59.123Z"));
        assert!(!valid_utc_date_time("2023-02-29T23:59:59Z"));
        assert!(!valid_utc_date_time("2024-01-01T24:00:00Z"));
    }
}
