use crate::error::PipelineError;
use crate::llm::{LlmClient, LlmMessage, LlmRequest};
use crate::types::{BugTicket, BuildOutput, PolicyYaml, RefinedSpec, TargetLanguage};

const ENGINEER_RUST_PROMPT: &str = r#"You are a Senior Backend Engineer. You write functions that compile to wasm32-wasi Components and run inside a Wasmtime sandbox via Wassette.

Target: Rust -> WebAssembly Component Model via cargo-component.

You MUST use the WASM Component Model with wit_bindgen. Your code MUST:
1. Use `wit_bindgen::generate!` macro to generate bindings from the WIT world
2. Implement the `Guest` trait on a `Component` struct
3. Export via `export!(Component);`
4. The WIT world MUST be named `girt-tool` and define exactly:
   `export run: func(input: string) -> result<string, string>;`

The `run` function receives a JSON string as input and returns a JSON string as output (or an error string).

EXAMPLE source_code:
```
wit_bindgen::generate!({
    world: "girt-tool",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        // Parse input JSON, do work, return output JSON
        let parsed: serde_json::Value = serde_json::from_str(&input)
            .map_err(|e| format!("Invalid input: {e}"))?;
        let result = serde_json::json!({"result": "value"});
        serde_json::to_string(&result).map_err(|e| format!("Serialization error: {e}"))
    }
}

export!(Component);
```

EXAMPLE wit_definition (use this EXACTLY, do NOT modify):
```
package girt:tool@0.1.0;

world girt-tool {
    export run: func(input: string) -> result<string, string>;
}
```

Environment Constraints:
- No local filesystem access unless explicitly granted in the spec.
- No native network access. Use WASI HTTP for outbound calls.
- Network access is restricted to hosts listed in the spec's constraints.
- SECRETS: Never hardcode credentials. Call host_auth_proxy(service_name) to get authenticated responses.
- Available crate dependencies: wit-bindgen, serde, serde_json.

Output ONLY valid JSON in this exact format:
{
  "source_code": "// Full Rust source code using wit_bindgen::generate! as shown above",
  "wit_definition": "package girt:tool@0.1.0;\n\nworld girt-tool {\n    export run: func(input: string) -> result<string, string>;\n}",
  "policy_yaml": "// Wassette policy YAML here",
  "language": "rust"
}

Do not include any text outside the JSON object. Do not use markdown code fences."#;

const ENGINEER_GO_PROMPT: &str = r#"You are a Senior Backend Engineer. You write functions that compile to wasm32-wasi Components using TinyGo and run inside a Wasmtime sandbox via Wassette.

Target: Go (TinyGo) -> WebAssembly Component Model with WIT interface definitions.

TinyGo Constraints:
- Use TinyGo-compatible standard library only. No cgo, no unsafe, no reflect.
- Import "unsafe" is forbidden.
- Use wasi-go bindings for WASI interfaces.
- Keep allocations minimal; TinyGo has a simple GC.

Environment Constraints:
- No local filesystem access unless explicitly granted in the spec.
- No native network access. Use WASI HTTP for outbound calls.
- Network access is restricted to hosts listed in the spec's constraints.
- SECRETS: Never hardcode credentials. Call host_auth_proxy(service_name) to get authenticated responses.

Output ONLY valid JSON in this exact format:
{
  "source_code": "// Full Go source code here",
  "wit_definition": "// WIT interface here",
  "policy_yaml": "// Wassette policy YAML here",
  "language": "go"
}

Do not include any text outside the JSON object. Do not use markdown code fences."#;

const ENGINEER_AS_PROMPT: &str = r#"You are a Senior Backend Engineer. You write functions that compile to wasm32-wasi Components using AssemblyScript and run inside a Wasmtime sandbox via Wassette.

Target: AssemblyScript -> WebAssembly Component Model with WIT interface definitions.

AssemblyScript Constraints:
- Use AssemblyScript standard library (as-*).
- No dynamic imports or eval.
- Use typed arrays and explicit memory management.
- Prefer static dispatch over dynamic dispatch.

Environment Constraints:
- No local filesystem access unless explicitly granted in the spec.
- No native network access. Use WASI HTTP for outbound calls.
- Network access is restricted to hosts listed in the spec's constraints.
- SECRETS: Never hardcode credentials. Call host_auth_proxy(service_name) to get authenticated responses.

Output ONLY valid JSON in this exact format:
{
  "source_code": "// Full AssemblyScript source code here",
  "wit_definition": "// WIT interface here",
  "policy_yaml": "// Wassette policy YAML here",
  "language": "assemblyscript"
}

Do not include any text outside the JSON object. Do not use markdown code fences."#;

const ENGINEER_FIX_PROMPT: &str = r#"You previously built a WASM component that had issues. Fix the code based on the bug ticket below.

Output ONLY the complete fixed code in the same JSON format as before:
{
  "source_code": "// Fixed source code",
  "wit_definition": "// WIT interface (may be unchanged)",
  "policy_yaml": "// Wassette policy YAML (may be unchanged)",
  "language": "<same language as before>"
}"#;

/// The Engineer agent generates WASM Component source code from the
/// Architect's refined spec. Supports Rust, Go (TinyGo), and AssemblyScript targets.
pub struct EngineerAgent<'a> {
    llm: &'a dyn LlmClient,
    target: TargetLanguage,
}

impl<'a> EngineerAgent<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        Self {
            llm,
            target: TargetLanguage::default(),
        }
    }

    pub fn with_target(llm: &'a dyn LlmClient, target: TargetLanguage) -> Self {
        Self { llm, target }
    }

    /// Get the system prompt for the configured target language.
    fn system_prompt(&self) -> &'static str {
        match self.target {
            TargetLanguage::Rust => ENGINEER_RUST_PROMPT,
            TargetLanguage::Go => ENGINEER_GO_PROMPT,
            TargetLanguage::AssemblyScript => ENGINEER_AS_PROMPT,
        }
    }

    /// Generate initial code from a refined spec.
    pub async fn build(&self, spec: &RefinedSpec) -> Result<BuildOutput, PipelineError> {
        let spec_json = serde_json::to_string_pretty(spec)
            .map_err(|e| PipelineError::LlmError(format!("Failed to serialize spec: {e}")))?;

        let request = LlmRequest {
            system_prompt: self.system_prompt().into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: format!("Implement this tool spec as a WASM Component:\n\n{spec_json}"),
            }],
            max_tokens: 4000,
        };

        let response = self.llm.chat(&request).await?;
        self.parse_build_output(&response.content, spec)
    }

    /// Fix code based on a bug ticket.
    pub async fn fix(
        &self,
        spec: &RefinedSpec,
        previous_output: &BuildOutput,
        ticket: &BugTicket,
    ) -> Result<BuildOutput, PipelineError> {
        let ticket_json = serde_json::to_string_pretty(ticket)
            .map_err(|e| PipelineError::LlmError(format!("Failed to serialize ticket: {e}")))?;

        let request = LlmRequest {
            system_prompt: ENGINEER_FIX_PROMPT.into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: format!(
                    "Original spec:\n{}\n\nPrevious code:\n{}\n\nBug ticket:\n{}",
                    serde_json::to_string_pretty(spec).unwrap_or_default(),
                    previous_output.source_code,
                    ticket_json,
                ),
            }],
            max_tokens: 4000,
        };

        let response = self.llm.chat(&request).await?;
        self.parse_build_output(&response.content, spec)
    }

    fn parse_build_output(
        &self,
        raw: &str,
        spec: &RefinedSpec,
    ) -> Result<BuildOutput, PipelineError> {
        // Try parsing as JSON first
        if let Ok(output) = serde_json::from_str::<BuildOutput>(raw) {
            return Ok(output);
        }

        // If JSON parsing fails, generate a policy.yaml from the spec and
        // treat the response as raw source code
        let policy = PolicyYaml::from_spec(&spec.spec);
        let policy_yaml = serde_json::to_string_pretty(&policy).unwrap_or_default();

        tracing::warn!(
            language = %self.target,
            "Engineer response was not valid JSON, treating as raw source code"
        );
        Ok(BuildOutput {
            source_code: raw.to_string(),
            wit_definition: String::new(),
            policy_yaml,
            language: self.target.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::StubLlmClient;
    use crate::types::SpecAction;
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_refined_spec() -> RefinedSpec {
        RefinedSpec {
            action: SpecAction::Build,
            spec: CapabilitySpec {
                name: "temp_convert".into(),
                description: "Convert temperature units".into(),
                inputs: serde_json::json!({"value": "f64", "from": "string", "to": "string"}),
                outputs: serde_json::json!({"result": "f64"}),
                constraints: CapabilityConstraints::default(),
            },
            design_notes: "Simple stateless conversion".into(),
            extend_target: None,
            extend_features: None,
        }
    }

    #[tokio::test]
    async fn builds_from_valid_json_response() {
        let response = serde_json::json!({
            "source_code": "fn convert(value: f64) -> f64 { value * 1.8 + 32.0 }",
            "wit_definition": "package temp:convert;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = EngineerAgent::new(&client);
        let spec = make_refined_spec();

        let output = agent.build(&spec).await.unwrap();
        assert_eq!(output.language, "rust");
        assert!(output.source_code.contains("convert"));
    }

    #[tokio::test]
    async fn handles_non_json_response_gracefully() {
        let client = StubLlmClient::constant("fn convert() { /* raw code */ }");
        let agent = EngineerAgent::new(&client);
        let spec = make_refined_spec();

        let output = agent.build(&spec).await.unwrap();
        assert!(output.source_code.contains("raw code"));
        assert_eq!(output.language, "rust");
    }

    #[tokio::test]
    async fn go_target_uses_go_language() {
        let client = StubLlmClient::constant("package main\nfunc convert() {}");
        let agent = EngineerAgent::with_target(&client, TargetLanguage::Go);
        let spec = make_refined_spec();

        let output = agent.build(&spec).await.unwrap();
        assert_eq!(output.language, "go");
        assert!(output.source_code.contains("package main"));
    }

    #[tokio::test]
    async fn assemblyscript_target_uses_as_language() {
        let client = StubLlmClient::constant("export function convert(): f64 { return 0; }");
        let agent = EngineerAgent::with_target(&client, TargetLanguage::AssemblyScript);
        let spec = make_refined_spec();

        let output = agent.build(&spec).await.unwrap();
        assert_eq!(output.language, "assemblyscript");
    }

    #[tokio::test]
    async fn go_json_response_parses_correctly() {
        let response = serde_json::json!({
            "source_code": "package main\nfunc Convert(v float64) float64 { return v * 1.8 + 32.0 }",
            "wit_definition": "package temp:convert;",
            "policy_yaml": "version: \"1.0\"",
            "language": "go"
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = EngineerAgent::with_target(&client, TargetLanguage::Go);
        let spec = make_refined_spec();

        let output = agent.build(&spec).await.unwrap();
        assert_eq!(output.language, "go");
        assert!(output.source_code.contains("Convert"));
    }
}
