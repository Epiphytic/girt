use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use crate::decision::{Decision, DeferTarget};
use crate::error::DecisionError;
use crate::spec::GateInput;

use super::DecisionLayer;

/// Similarity threshold for considering two specs as matching.
/// 0.0 = no similarity, 1.0 = identical.
const SIMILARITY_THRESHOLD: f64 = 0.45;

/// A known tool spec for similarity comparison.
#[derive(Debug, Clone)]
pub struct KnownSpec {
    pub name: String,
    pub description: String,
    pub keywords: HashSet<String>,
}

impl KnownSpec {
    pub fn new(name: &str, description: &str) -> Self {
        let desc_keywords = extract_keywords(description);
        let name_keywords = extract_keywords(name);
        let keywords: HashSet<String> = desc_keywords.union(&name_keywords).cloned().collect();
        Self {
            name: name.into(),
            description: description.into(),
            keywords,
        }
    }
}

/// Similarity check layer. Compares incoming capability specs against
/// known tool specs using keyword-based Jaccard similarity.
///
/// Future enhancement: replace keyword matching with embedding-based
/// similarity using a vector store.
pub struct SimilarityLayer {
    known_specs: Vec<KnownSpec>,
}

impl SimilarityLayer {
    pub fn new(known_specs: Vec<KnownSpec>) -> Self {
        Self { known_specs }
    }

    /// Find the best matching known spec above the similarity threshold.
    fn find_match(&self, name: &str, description: &str) -> Option<(&KnownSpec, f64)> {
        let input_keywords = extract_keywords(description);
        let input_name_keywords = extract_keywords(name);
        let combined: HashSet<String> = input_keywords
            .union(&input_name_keywords)
            .cloned()
            .collect();

        let mut best_match: Option<(&KnownSpec, f64)> = None;

        for spec in &self.known_specs {
            // Check exact name match first
            if spec.name == name {
                return Some((spec, 1.0));
            }

            let score = jaccard_similarity(&combined, &spec.keywords);

            if score >= SIMILARITY_THRESHOLD
                && (best_match.is_none() || score > best_match.unwrap().1)
            {
                best_match = Some((spec, score));
            }
        }

        best_match
    }
}

impl DecisionLayer for SimilarityLayer {
    fn name(&self) -> &str {
        "similarity"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>> {
        Box::pin(async move {
            let spec = match input {
                GateInput::Creation(spec) => spec,
                GateInput::Execution(_) => return Ok(None), // Skip for execution
            };

            if let Some((matched, score)) = self.find_match(&spec.name, &spec.description) {
                tracing::info!(
                    input_name = %spec.name,
                    matched_name = %matched.name,
                    score = score,
                    "Similarity match found"
                );

                Ok(Some(Decision::Defer {
                    target: DeferTarget::ExtendTool {
                        tool_name: matched.name.clone(),
                        suggested_features: vec![],
                    },
                }))
            } else {
                Ok(None) // No match, pass through
            }
        })
    }
}

/// Extract keywords from text by splitting on whitespace and punctuation,
/// lowercasing, and filtering stop words.
fn extract_keywords(text: &str) -> HashSet<String> {
    let stop_words: HashSet<&str> = [
        "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
        "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do",
        "does", "did", "will", "would", "could", "should", "may", "might", "can", "this", "that",
        "these", "those", "it", "its", "not", "no", "nor", "so", "if", "then", "than", "when",
        "what", "which", "who", "how", "all", "each", "every", "both", "few", "more", "most",
        "other", "some", "such", "only", "own", "same", "into", "over", "after", "before",
    ]
    .into_iter()
    .collect();

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 2 && !stop_words.contains(w))
        .map(String::from)
        .collect()
}

/// Jaccard similarity coefficient between two sets.
/// Returns 0.0 if both sets are empty.
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    intersection / union
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_known_specs() -> Vec<KnownSpec> {
        vec![
            KnownSpec::new(
                "github_api",
                "Query and manage GitHub issues, pull requests, and repositories",
            ),
            KnownSpec::new(
                "http_client",
                "General purpose HTTP client for GET POST PUT DELETE requests",
            ),
            KnownSpec::new(
                "json_transform",
                "Parse query and transform JSON data using JSONPath expressions",
            ),
        ]
    }

    fn make_creation_input(name: &str, description: &str) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: description.into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    #[tokio::test]
    async fn exact_name_match_defers() {
        let layer = SimilarityLayer::new(make_known_specs());
        let input = make_creation_input("github_api", "fetch issues from GitHub");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), Decision::Defer { .. }));
    }

    #[tokio::test]
    async fn similar_description_defers() {
        let layer = SimilarityLayer::new(make_known_specs());
        let input = make_creation_input(
            "fetch_github_issues",
            "Query GitHub issues and pull requests for a repository",
        );

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), Decision::Defer { .. }));
    }

    #[tokio::test]
    async fn unrelated_spec_passes_through() {
        let layer = SimilarityLayer::new(make_known_specs());
        let input = make_creation_input(
            "weather_forecast",
            "Get weather forecast data for a geographic location",
        );

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execution_requests_pass_through() {
        let layer = SimilarityLayer::new(make_known_specs());
        let input = GateInput::Execution(crate::spec::ExecutionRequest {
            tool_name: "github_api".into(),
            arguments: serde_json::Value::Null,
        });

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn keyword_extraction() {
        let keywords = extract_keywords("Query GitHub issues and pull requests for a repository");
        assert!(keywords.contains("query"));
        assert!(keywords.contains("github"));
        assert!(keywords.contains("issues"));
        assert!(keywords.contains("pull"));
        assert!(keywords.contains("requests"));
        assert!(keywords.contains("repository"));
        // Stop words filtered
        assert!(!keywords.contains("and"));
        assert!(!keywords.contains("for"));
    }

    #[test]
    fn jaccard_identical() {
        let a: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_disjoint() {
        let a: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["baz", "qux"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &b)).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_empty() {
        let a: HashSet<String> = HashSet::new();
        let b: HashSet<String> = HashSet::new();
        assert!((jaccard_similarity(&a, &b)).abs() < f64::EPSILON);
    }
}
