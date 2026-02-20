pub mod architect;
pub mod engineer;
pub mod qa;
pub mod red_team;

/// Extract a JSON object from an LLM response that may contain markdown
/// code fences or surrounding text.
///
/// Tries in order:
/// 1. Direct parse of the entire string
/// 2. Extract from ```json ... ``` or ``` ... ``` code fences
/// 3. Find the first `{` ... last `}` and parse that substring
pub(crate) fn extract_json<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    // 1. Try direct parse
    if let Ok(val) = serde_json::from_str::<T>(raw) {
        return Some(val);
    }

    // 2. Try extracting from code fences
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```") {
        // Find the end of the opening fence line
        let after_fence = &trimmed[start + 3..];
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_fence[content_start..];
        // Find the closing fence
        if let Some(end) = content.find("```") {
            let json_str = content[..end].trim();
            if let Ok(val) = serde_json::from_str::<T>(json_str) {
                return Some(val);
            }
        }
    }

    // 3. Try finding the outermost { ... }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let json_str = &trimmed[start..=end];
                if let Ok(val) = serde_json::from_str::<T>(json_str) {
                    return Some(val);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_direct_json() {
        let raw = r#"{"key": "value"}"#;
        let val: serde_json::Value = extract_json(raw).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn extracts_from_code_fence() {
        let raw = "Here is the result:\n```json\n{\"key\": \"value\"}\n```\nDone.";
        let val: serde_json::Value = extract_json(raw).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn extracts_from_surrounding_text() {
        let raw = "Sure, here is the JSON:\n{\"key\": \"value\"}\nHope that helps!";
        let val: serde_json::Value = extract_json(raw).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn returns_none_for_non_json() {
        let raw = "This is just text with no JSON";
        let val: Option<serde_json::Value> = extract_json(raw);
        assert!(val.is_none());
    }
}
