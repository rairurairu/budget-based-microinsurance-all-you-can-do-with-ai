//! The JSON Schema a component's reply must match, sent as the structured output format and
//! published in `contracts/schemas/ai/` (`just gen`). Generated from the Rust reply type, then
//! reduced to what structured outputs accept: every object closed and every property required
//! (an optional field is nullable instead), no numeric or length constraints, only the
//! supported string formats.
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

const FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "uri",
    "ipv4",
    "ipv6",
    "uuid",
];

const DROPPED: &[&str] = &[
    "$schema",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "uniqueItems",
    "pattern",
    "default",
];

fn reduce(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for k in DROPPED {
                map.remove(*k);
            }
            if map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|f| !FORMATS.contains(&f))
            {
                map.remove("format");
            }
            if let Some(Value::Object(props)) = map.get("properties") {
                let names: Vec<Value> = props.keys().cloned().map(Value::String).collect();
                map.insert("required".into(), Value::Array(names));
                map.insert("additionalProperties".into(), Value::Bool(false));
            }
            for (_, child) in map.iter_mut() {
                reduce(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(reduce),
        _ => {}
    }
}

/// The reply schema for `T`, inlined and reduced.
pub fn output_schema<T: JsonSchema>() -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|s| {
            s.inline_subschemas = true;
            s.meta_schema = None;
        })
        .into_generator();
    let mut schema = generator.root_schema_for::<T>().to_value();
    reduce(&mut schema);
    if let Value::Object(map) = &mut schema {
        map.remove("$defs");
    }
    schema
}

/// The reduced schema with a title, for publishing.
pub fn published<T: JsonSchema>(title: &str) -> Value {
    let mut schema = output_schema::<T>();
    if let Value::Object(map) = &mut schema {
        let mut out = Map::new();
        out.insert("title".into(), Value::String(title.into()));
        map.remove("title");
        out.extend(std::mem::take(map));
        *map = out;
    }
    schema
}
