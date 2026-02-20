use girt_core::spec::{CapabilityConstraints, CapabilitySpec};

/// Standard library tool specifications.
///
/// These are pre-designed, reusable tool specs that cover common capabilities.
/// The Architect can reference these when refining capability requests, and
/// the Registry Lookup layer checks against these before triggering new builds.
pub fn standard_library() -> Vec<CapabilitySpec> {
    vec![
        http_client(),
        json_transform(),
        file_io(),
        github_api(),
        gitlab_api(),
        text_processing(),
        crypto_hash(),
        csv_parser(),
    ]
}

/// HTTP client for making authenticated requests to allowed hosts.
pub fn http_client() -> CapabilitySpec {
    CapabilitySpec {
        name: "http_client".into(),
        description: "General-purpose HTTP client for making GET, POST, PUT, DELETE requests \
                      to allowed hosts with JSON request/response handling."
            .into(),
        inputs: serde_json::json!({
            "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"], "required": true},
            "url": {"type": "string", "required": true},
            "headers": {"type": "object", "description": "Additional headers"},
            "body": {"type": "object", "description": "Request body (for POST/PUT/PATCH)"},
            "auth_service": {"type": "string", "description": "Service name for host_auth_proxy credential injection"}
        }),
        outputs: serde_json::json!({
            "status": {"type": "integer"},
            "headers": {"type": "object"},
            "body": {"type": "object"}
        }),
        constraints: CapabilityConstraints {
            network: vec![], // Populated per-instance by the Architect
            storage: vec![],
            secrets: vec![],
        },
    }
}

/// JSON parsing and transformation tool.
pub fn json_transform() -> CapabilitySpec {
    CapabilitySpec {
        name: "json_transform".into(),
        description: "Parse, query, and transform JSON data using JSONPath expressions. \
                      Supports filtering, mapping, flattening, and restructuring."
            .into(),
        inputs: serde_json::json!({
            "data": {"type": "object", "description": "Input JSON data", "required": true},
            "query": {"type": "string", "description": "JSONPath expression to extract data"},
            "transform": {"type": "object", "description": "Transformation spec (map, filter, flatten, pick, omit)"}
        }),
        outputs: serde_json::json!({
            "result": {"type": "object", "description": "Transformed output"}
        }),
        constraints: CapabilityConstraints::default(),
    }
}

/// File I/O within granted paths.
pub fn file_io() -> CapabilitySpec {
    CapabilitySpec {
        name: "file_io".into(),
        description: "Read and write files within granted storage paths. Supports text and \
                      binary operations, directory listing, and file metadata."
            .into(),
        inputs: serde_json::json!({
            "operation": {"type": "string", "enum": ["read", "write", "append", "list", "stat", "delete"], "required": true},
            "path": {"type": "string", "description": "File path (must be within allowed storage paths)", "required": true},
            "content": {"type": "string", "description": "Content to write (for write/append operations)"},
            "encoding": {"type": "string", "description": "Text encoding (default: utf-8)"}
        }),
        outputs: serde_json::json!({
            "content": {"type": "string", "description": "File content (for read)"},
            "entries": {"type": "array", "description": "Directory entries (for list)"},
            "metadata": {"type": "object", "description": "File metadata (for stat)"},
            "success": {"type": "boolean"}
        }),
        constraints: CapabilityConstraints {
            network: vec![],
            storage: vec![], // Populated per-instance
            secrets: vec![],
        },
    }
}

/// GitHub API integration.
pub fn github_api() -> CapabilitySpec {
    CapabilitySpec {
        name: "github_api".into(),
        description: "Query and manage GitHub resources including issues, pull requests, \
                      repositories, and actions. Supports filtering, pagination, and CRUD."
            .into(),
        inputs: serde_json::json!({
            "resource": {"type": "string", "enum": ["issues", "pulls", "repos", "actions", "releases"], "required": true},
            "owner": {"type": "string", "required": true},
            "repo": {"type": "string", "required": true},
            "action": {"type": "string", "enum": ["list", "get", "create", "update", "close"], "default": "list"},
            "filters": {"type": "object", "description": "Resource-specific filters (state, labels, etc.)"},
            "page": {"type": "integer", "default": 1},
            "per_page": {"type": "integer", "default": 30}
        }),
        outputs: serde_json::json!({
            "items": {"type": "array"},
            "total_count": {"type": "integer"},
            "next_page": {"type": "integer"}
        }),
        constraints: CapabilityConstraints {
            network: vec!["api.github.com".into()],
            storage: vec![],
            secrets: vec!["GITHUB_TOKEN".into()],
        },
    }
}

/// GitLab API integration.
pub fn gitlab_api() -> CapabilitySpec {
    CapabilitySpec {
        name: "gitlab_api".into(),
        description: "Query and manage GitLab resources including issues, merge requests, \
                      projects, and pipelines. Supports filtering and pagination."
            .into(),
        inputs: serde_json::json!({
            "resource": {"type": "string", "enum": ["issues", "merge_requests", "projects", "pipelines"], "required": true},
            "project_id": {"type": "string", "required": true},
            "action": {"type": "string", "enum": ["list", "get", "create", "update"], "default": "list"},
            "filters": {"type": "object"},
            "page": {"type": "integer", "default": 1},
            "per_page": {"type": "integer", "default": 20}
        }),
        outputs: serde_json::json!({
            "items": {"type": "array"},
            "total_count": {"type": "integer"},
            "next_page": {"type": "integer"}
        }),
        constraints: CapabilityConstraints {
            network: vec!["gitlab.com".into()],
            storage: vec![],
            secrets: vec!["GITLAB_TOKEN".into()],
        },
    }
}

/// Text processing tool.
pub fn text_processing() -> CapabilitySpec {
    CapabilitySpec {
        name: "text_processing".into(),
        description: "Text manipulation including regex matching, replacement, splitting, \
                      joining, case conversion, trimming, and encoding/decoding."
            .into(),
        inputs: serde_json::json!({
            "operation": {"type": "string", "enum": ["regex_match", "regex_replace", "split", "join", "case_convert", "trim", "encode", "decode", "template"], "required": true},
            "text": {"type": "string", "required": true},
            "pattern": {"type": "string", "description": "Regex pattern (for regex ops)"},
            "replacement": {"type": "string", "description": "Replacement string (for replace)"},
            "separator": {"type": "string", "description": "Separator (for split/join)"},
            "case": {"type": "string", "enum": ["upper", "lower", "title", "snake", "camel", "kebab"]},
            "encoding": {"type": "string", "enum": ["base64", "url", "html"]}
        }),
        outputs: serde_json::json!({
            "result": {"type": "string"},
            "matches": {"type": "array", "description": "Regex matches (for regex_match)"},
            "parts": {"type": "array", "description": "Split parts (for split)"}
        }),
        constraints: CapabilityConstraints::default(),
    }
}

/// Cryptographic hash tool.
pub fn crypto_hash() -> CapabilitySpec {
    CapabilitySpec {
        name: "crypto_hash".into(),
        description: "Compute cryptographic hashes (SHA-256, SHA-512, MD5, BLAKE3) and \
                      HMAC signatures for data integrity verification."
            .into(),
        inputs: serde_json::json!({
            "operation": {"type": "string", "enum": ["hash", "hmac", "verify"], "required": true},
            "data": {"type": "string", "required": true},
            "algorithm": {"type": "string", "enum": ["sha256", "sha512", "md5", "blake3"], "default": "sha256"},
            "key": {"type": "string", "description": "HMAC key (for hmac operation)"},
            "expected_hash": {"type": "string", "description": "Hash to verify against (for verify)"}
        }),
        outputs: serde_json::json!({
            "hash": {"type": "string", "description": "Hex-encoded hash"},
            "verified": {"type": "boolean", "description": "Verification result (for verify)"}
        }),
        constraints: CapabilityConstraints::default(),
    }
}

/// CSV parsing and generation tool.
pub fn csv_parser() -> CapabilitySpec {
    CapabilitySpec {
        name: "csv_parser".into(),
        description:
            "Parse CSV data into structured records and generate CSV from structured data. \
                      Supports custom delimiters, headers, and encoding."
                .into(),
        inputs: serde_json::json!({
            "operation": {"type": "string", "enum": ["parse", "generate"], "required": true},
            "data": {"type": "string", "description": "CSV text (for parse)"},
            "records": {"type": "array", "description": "Records to convert to CSV (for generate)"},
            "delimiter": {"type": "string", "default": ","},
            "has_headers": {"type": "boolean", "default": true},
            "columns": {"type": "array", "description": "Column names to select (for parse)"}
        }),
        outputs: serde_json::json!({
            "records": {"type": "array", "description": "Parsed records (for parse)"},
            "csv": {"type": "string", "description": "Generated CSV (for generate)"},
            "headers": {"type": "array", "description": "Column headers"},
            "row_count": {"type": "integer"}
        }),
        constraints: CapabilityConstraints::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_library_has_expected_tools() {
        let stdlib = standard_library();
        assert_eq!(stdlib.len(), 8);

        let names: Vec<&str> = stdlib.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"http_client"));
        assert!(names.contains(&"json_transform"));
        assert!(names.contains(&"file_io"));
        assert!(names.contains(&"github_api"));
        assert!(names.contains(&"gitlab_api"));
        assert!(names.contains(&"text_processing"));
        assert!(names.contains(&"crypto_hash"));
        assert!(names.contains(&"csv_parser"));
    }

    #[test]
    fn github_api_has_correct_constraints() {
        let spec = github_api();
        assert_eq!(spec.constraints.network, vec!["api.github.com"]);
        assert_eq!(spec.constraints.secrets, vec!["GITHUB_TOKEN"]);
        assert!(spec.constraints.storage.is_empty());
    }

    #[test]
    fn stateless_tools_have_no_constraints() {
        let spec = json_transform();
        assert!(spec.constraints.network.is_empty());
        assert!(spec.constraints.storage.is_empty());
        assert!(spec.constraints.secrets.is_empty());
    }

    #[test]
    fn all_specs_have_names_and_descriptions() {
        for spec in standard_library() {
            assert!(!spec.name.is_empty(), "Spec has empty name");
            assert!(
                !spec.description.is_empty(),
                "Spec {} has empty description",
                spec.name
            );
        }
    }
}
