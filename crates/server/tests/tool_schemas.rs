//! Every MCP tool's input schema must be one clients accept: plain JSON
//! schema objects all the way down, never a bare `true` or `false`.

fn check(schema: &serde_json::Value, path: &str, bad: &mut Vec<String>) {
    match schema {
        serde_json::Value::Bool(_) => bad.push(path.to_owned()),
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match key.as_str() {
                    "properties" | "$defs" | "definitions" => {
                        if let Some(children) = value.as_object() {
                            for (name, child) in children {
                                check(child, &format!("{path}.{key}.{name}"), bad);
                            }
                        }
                    }
                    "items" | "additionalProperties" if value.is_object() => {
                        check(value, &format!("{path}.{key}"), bad);
                    }
                    "anyOf" | "oneOf" | "allOf" => {
                        for (index, child) in value.as_array().into_iter().flatten().enumerate() {
                            check(child, &format!("{path}.{key}[{index}]"), bad);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[test]
fn every_tool_schema_is_a_plain_object_schema() {
    let mut bad = Vec::new();
    for tool in hoista_server::mcp::tools() {
        let name = tool["name"].as_str().unwrap_or("?").to_owned();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        check(&tool["inputSchema"], &name, &mut bad);
    }
    assert!(bad.is_empty(), "schemas clients reject: {bad:?}");
}
