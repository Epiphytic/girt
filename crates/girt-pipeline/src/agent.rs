pub mod architect;
pub mod engineer;
pub mod qa;
pub mod red_team;

/// Extract a JSON object from an LLM response that may contain markdown
/// code fences, `<think>` blocks, or surrounding text.
///
/// Tries in order:
/// 1. Direct parse of the entire string
/// 2. Strip `<think>...</think>` blocks, then try direct parse
/// 3. Extract from ```json ... ``` or ``` ... ``` code fences
/// 4. Find the first `{` ... last `}` and parse that substring
pub(crate) fn extract_json<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    // 1. Try direct parse
    if let Ok(val) = serde_json::from_str::<T>(raw) {
        return Some(val);
    }

    // 2. Strip <think>...</think> blocks (common in reasoning models)
    let cleaned = strip_think_blocks(raw);
    let trimmed = cleaned.trim();

    if let Ok(val) = serde_json::from_str::<T>(trimmed) {
        return Some(val);
    }

    // 3. Try extracting from code fences (search from end to avoid
    //    picking up backticks inside think blocks or prose)
    if let Some(json_str) = extract_from_code_fence(trimmed)
        && let Ok(val) = serde_json::from_str::<T>(json_str)
    {
        return Some(val);
    }

    // 4. Try finding the outermost { ... }
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let json_str = &trimmed[start..=end];
        if let Ok(val) = serde_json::from_str::<T>(json_str) {
            return Some(val);
        }
    }

    None
}

/// Remove `<think>...</think>` blocks from LLM output.
fn strip_think_blocks(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[start + end + 8..];
        } else {
            // Unclosed <think> tag -- strip everything after it
            return result;
        }
    }
    result.push_str(remaining);
    result
}

/// Extract content from the last ```json ... ``` or ``` ... ``` code fence.
fn extract_from_code_fence(s: &str) -> Option<&str> {
    // Find all ``` positions
    let fence_positions: Vec<usize> = s
        .match_indices("```")
        .map(|(pos, _)| pos)
        .collect();

    // Need at least 2 fences (opening + closing)
    if fence_positions.len() < 2 {
        return None;
    }

    // Try the last pair of fences first (most likely to be the JSON output)
    for pair in fence_positions.windows(2).rev() {
        let open = pair[0];
        let close = pair[1];

        // Skip the opening fence line (e.g., ```json\n)
        let after_fence = &s[open + 3..];
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content_offset = open + 3 + content_start;

        if content_offset < close {
            let content = s[content_offset..close].trim();
            // Quick check: does it look like JSON?
            if content.starts_with('{') {
                return Some(content);
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

    #[test]
    fn extracts_json_after_think_block() {
        let raw = "<think>Let me analyze `this` and think about the `best` approach.\nI'll use a simple design.</think>\n{\"key\": \"value\"}";
        let val: serde_json::Value = extract_json(raw).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn extracts_json_from_code_fence_after_think() {
        let raw = "<think>Some reasoning with `backticks` inside.</think>\n```json\n{\"key\": \"value\"}\n```";
        let val: serde_json::Value = extract_json(raw).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn strip_think_blocks_removes_tags() {
        let result = strip_think_blocks("before<think>hidden</think>after");
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn strip_think_blocks_handles_no_tags() {
        let result = strip_think_blocks("no tags here");
        assert_eq!(result, "no tags here");
    }
}
