//! State Patch（JSON Merge Patch，RFC 7386）与更新预算。
//!
//! v1 语义：插件提交 JSON Merge Patch，宿主在副本上原子应用并对完整新 State 做
//! schema 校验，成功后替换；失败时旧 State 不变。预算常量是 P0 初始值，在
//! evil/clock/10-instance 数据后冻结。

/// 完整 State 最大字节数（ui-ir-v1 §10）。
pub const MAX_STATE_BYTES: usize = 64 * 1024;
/// 单 State Patch 最大字节数。
pub const MAX_PATCH_BYTES: usize = 16 * 1024;
/// 每实例每秒 UI 更新上限。
pub const MAX_UPDATE_RATE_PER_SEC: u32 = 30;

/// 应用 JSON Merge Patch（RFC 7386）。
///
/// 对象 patch 递归合并；`null` 值删除键；非对象 patch 整体替换。
pub fn merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match patch {
        serde_json::Value::Object(patch_obj) => {
            if !target.is_object() {
                *target = serde_json::Value::Object(serde_json::Map::new());
            }
            if let serde_json::Value::Object(target_obj) = target {
                for (key, value) in patch_obj {
                    if value.is_null() {
                        target_obj.remove(key);
                    } else {
                        let entry = target_obj.entry(key).or_insert(serde_json::Value::Null);
                        merge_patch(entry, value);
                    }
                }
            }
        }
        non_object => {
            *target = non_object.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_nested_object_and_adds_fields() {
        let mut state = json!({"time": "00:00:00", "nested": {"a": 1}});
        merge_patch(
            &mut state,
            &json!({"time": "12:00:00", "nested": {"b": 2}, "new": true}),
        );
        assert_eq!(
            state,
            json!({"time": "12:00:00", "nested": {"a": 1, "b": 2}, "new": true})
        );
    }

    #[test]
    fn null_deletes_keys() {
        let mut state = json!({"keep": 1, "drop": 2});
        merge_patch(&mut state, &json!({"drop": null}));
        assert_eq!(state, json!({"keep": 1}));
    }

    #[test]
    fn non_object_patch_replaces_whole_state() {
        let mut state = json!({"a": 1});
        merge_patch(&mut state, &json!(42));
        assert_eq!(state, json!(42));
    }

    #[test]
    fn merge_does_not_mutate_the_patch() {
        let mut state = json!({"a": {"x": 1}});
        let patch = json!({"a": {"y": 2}});
        merge_patch(&mut state, &patch);
        assert_eq!(patch, json!({"a": {"y": 2}}));
    }
}
