//! Post-hoc validation of `Answer.extra` against a manifest-declared schema
//! (ARCHITECTURE §18.1, ROADMAP T3.4).
//!
//! `extra` is the narrow-waist escape hatch: agent-specific structure rides
//! there, **never load-bearing** for the contract. An agent definition may
//! declare a JSON schema for it; the agent validates the produced `extra`
//! against that schema *after* the run and surfaces any violations as
//! [`Answer::warnings`](crate::Answer) — never as a failure. A malformed extra
//! is a soft signal to the author, not a broken contract.
//!
//! This is a deliberately **minimal, dependency-free** validator covering the
//! JSON-Schema subset a declared answer-extra realistically uses: `type`
//! (object/array/string/number/integer/boolean/null), `required` and
//! `properties` for objects, and `items` for arrays (recursively). It is not a
//! full JSON-Schema implementation — unknown keywords are ignored, matching the
//! "advisory, never load-bearing" intent.

use serde_json::Value;

/// Validate `value` against `schema`, returning one human-readable message per
/// violation (empty when it conforms, or when `schema` uses only keywords this
/// minimal validator ignores).
pub fn validate_extra(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_node(schema, value, "extra", &mut errors);
    errors
}

fn validate_node(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if !matches_type(ty, value) {
            errors.push(format!(
                "{path}: expected type `{ty}`, got `{}`",
                type_name(value)
            ));
            // A type mismatch makes deeper checks meaningless.
            return;
        }
    }

    if let Some(obj) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for req in required {
                if let Some(key) = req.as_str() {
                    if !obj.contains_key(key) {
                        errors.push(format!("{path}: missing required property `{key}`"));
                    }
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (key, subschema) in props {
                if let Some(child) = obj.get(key) {
                    validate_node(subschema, child, &format!("{path}.{key}"), errors);
                }
            }
        }
    }

    if let Some(arr) = value.as_array() {
        if let Some(items) = schema.get("items") {
            for (i, child) in arr.iter().enumerate() {
                validate_node(items, child, &format!("{path}[{i}]"), errors);
            }
        }
    }
}

fn matches_type(ty: &str, value: &Value) -> bool {
    match ty {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        // JSON has no integer type; accept whole-valued numbers.
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        // Unknown type keyword: don't complain (advisory validator).
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn docs_schema() -> Value {
        json!({
            "type": "object",
            "required": ["related_documents"],
            "properties": {
                "related_documents": { "type": "array", "items": { "type": "string" } }
            }
        })
    }

    #[test]
    fn conforming_extra_has_no_violations() {
        let extra = json!({ "answer": "hi", "related_documents": ["a.md", "b.md"] });
        assert!(validate_extra(&docs_schema(), &extra).is_empty());
    }

    #[test]
    fn missing_required_and_bad_item_type_are_reported() {
        let extra = json!({ "related_documents": ["ok.md", 7] });
        let errs = validate_extra(&docs_schema(), &extra);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("related_documents[1]"), "{errs:?}");

        let missing = json!({ "other": true });
        let errs = validate_extra(&docs_schema(), &missing);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("missing required property `related_documents`"));
    }

    #[test]
    fn top_level_type_mismatch_short_circuits() {
        let errs = validate_extra(&docs_schema(), &json!("not an object"));
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("expected type `object`"), "{errs:?}");
    }

    #[test]
    fn unknown_keywords_are_ignored() {
        let schema = json!({ "type": "object", "additionalProperties": false, "minProperties": 2 });
        assert!(validate_extra(&schema, &json!({ "a": 1 })).is_empty());
    }
}
