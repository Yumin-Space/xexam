//! v1.3 sprint 2 (owner): an ordinary user (every identity that is not an administrator, `Identity::is_admin()`) never
//! learns which AI answers: no model, connection, provider, address or gateway in any JSON body or SSE frame sent to
//! them. Administrators keep every field. Stored data never changes; this only shapes what is sent.
use serde_json::Value;

/// Object keys that name the AI behind an answer; removed at any depth from what a non-administrator receives.
pub const HIDDEN_KEYS: [&str; 10] = [
    "model",
    "modelFallback",
    "connection",
    "connectionId",
    "connectionName",
    "pick",
    "chatDefault",
    "provider",
    "baseUrl",
    "host",
];

/// Removes [`HIDDEN_KEYS`] from `v` (objects at any depth; arrays are walked) unless `admin`. Key order of what stays is
/// kept (`Map::retain`; never `Map::remove`, which is `swap_remove` under `preserve_order`).
pub fn for_viewer(admin: bool, v: &mut Value) {
    if !admin {
        strip(v);
    }
}

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|k, _| !HIDDEN_KEYS.contains(&k.as_str()));
            m.values_mut().for_each(strip);
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "chatId": 1,
            "model": "demo-chat-small",
            "connection": "main",
            "latest": {"model": "demo-x", "content": "hi", "connection": "main"},
            "messages": [
                {"id": 1, "model": "demo-x", "connection": "main", "content": "a"},
                {"id": 2, "content": "b"}
            ]
        })
    }

    #[test]
    fn admin_keeps_everything() {
        let mut v = sample();
        for_viewer(true, &mut v);
        assert_eq!(v, sample());
    }

    #[test]
    fn user_loses_every_hidden_key_at_any_depth() {
        let mut v = sample();
        for_viewer(false, &mut v);
        let raw = v.to_string();
        assert!(
            !raw.contains("\"model\"") && !raw.contains("\"connection\""),
            "{raw}"
        );
        assert_eq!(v["latest"]["content"], "hi");
        assert_eq!(v["messages"][0]["content"], "a");
    }

    #[test]
    fn order_of_the_other_keys_is_kept() {
        let mut v = json!({
            "chatId": 1, "chatTitle": "t", "model": "m", "newChat": true, "connection": "c", "retry": false,
            "modelFallback": "f", "free": false
        });
        for_viewer(false, &mut v);
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["chatId", "chatTitle", "newChat", "retry", "free"]);
    }

    #[test]
    fn content_strings_are_untouched() {
        let mut v = json!({"content": "the model said \"connection\"", "model": "m"});
        for_viewer(false, &mut v);
        assert_eq!(v, json!({"content": "the model said \"connection\""}));
    }
}
