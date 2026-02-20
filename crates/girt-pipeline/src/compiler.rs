use std::path::{Path, PathBuf};

use crate::error::PipelineError;

pub struct CompileInput {
    pub source_code: String,
    pub wit_definition: String,
    pub tool_name: String,
    pub tool_version: String,
}

pub struct CompileOutput {
    pub wasm_path: PathBuf,
    pub build_dir: PathBuf,
}

pub struct WasmCompiler {
    cargo_component_bin: String,
}

impl WasmCompiler {
    pub fn new() -> Self {
        Self {
            cargo_component_bin: "cargo-component".into(),
        }
    }

    pub fn scaffold_project(
        &self,
        input: &CompileInput,
        base_dir: &Path,
    ) -> Result<PathBuf, PipelineError> {
        let project_dir = base_dir.join(&input.tool_name);
        std::fs::create_dir_all(project_dir.join("src"))?;
        std::fs::create_dir_all(project_dir.join("wit"))?;

        let cargo_toml = format!(
            r#"[package]
name = "{name}"
version = "{version}"
edition = "2024"

[dependencies]
wit-bindgen = "0.41"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "girt:tool@0.1.0"
"#,
            name = input.tool_name,
            version = input.tool_version,
        );
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

        std::fs::write(project_dir.join("src/lib.rs"), &input.source_code)?;

        std::fs::write(project_dir.join("wit/world.wit"), &input.wit_definition)?;

        Ok(project_dir)
    }

    pub async fn compile(&self, input: &CompileInput) -> Result<CompileOutput, PipelineError> {
        let tmp = tempfile::tempdir()?;
        let project_dir = self.scaffold_project(input, tmp.path())?;

        let output = tokio::process::Command::new(&self.cargo_component_bin)
            .arg("build")
            .arg("--release")
            .current_dir(&project_dir)
            .output()
            .await
            .map_err(|e| {
                PipelineError::CompilationError(format!(
                    "Failed to run cargo-component: {e}. Is it installed? (cargo install cargo-component)"
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(PipelineError::CompilationError(format!(
                "cargo-component build failed:\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        let wasm_dir = project_dir
            .join("target")
            .join("wasm32-wasip1")
            .join("release");

        let wasm_filename = format!("{}.wasm", input.tool_name.replace('-', "_"));
        let wasm_path = wasm_dir.join(&wasm_filename);

        if !wasm_path.exists() {
            let mut found = None;
            if wasm_dir.exists() {
                for entry in std::fs::read_dir(&wasm_dir)? {
                    let entry = entry?;
                    if entry.path().extension().is_some_and(|e| e == "wasm") {
                        found = Some(entry.path());
                        break;
                    }
                }
            }
            match found {
                Some(path) => {
                    return Ok(CompileOutput {
                        wasm_path: path,
                        build_dir: project_dir,
                    })
                }
                None => {
                    return Err(PipelineError::CompilationError(format!(
                        "No .wasm file found in {}",
                        wasm_dir.display()
                    )))
                }
            }
        }

        let _ = tmp.keep();

        Ok(CompileOutput {
            wasm_path,
            build_dir: project_dir,
        })
    }
}

impl Default for WasmCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scaffolds_cargo_project_correctly() {
        let tmp = TempDir::new().unwrap();
        let compiler = WasmCompiler::new();

        let input = CompileInput {
            source_code: "// placeholder".into(),
            wit_definition: "package test:tool;".into(),
            tool_name: "test_tool".into(),
            tool_version: "0.1.0".into(),
        };

        let build_dir = compiler.scaffold_project(&input, tmp.path()).unwrap();

        assert!(build_dir.join("Cargo.toml").exists());
        assert!(build_dir.join("src/lib.rs").exists());
        assert!(build_dir.join("wit/world.wit").exists());
    }

    #[tokio::test]
    #[ignore] // Requires cargo-component installed
    async fn compiles_minimal_wasm_component() {
        let compiler = WasmCompiler::new();

        let input = CompileInput {
            source_code: r#"
wit_bindgen::generate!({
    world: "girt-tool",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        Ok(format!("echo: {input}"))
    }
}

export!(Component);
"#
            .into(),
            wit_definition: r#"
package girt:tool@0.1.0;

world girt-tool {
    export run: func(input: string) -> result<string, string>;
}
"#
            .into(),
            tool_name: "echo_tool".into(),
            tool_version: "0.1.0".into(),
        };

        let output = compiler.compile(&input).await.unwrap();
        assert!(output.wasm_path.exists());
        assert!(output.wasm_path.extension().unwrap() == "wasm");
    }
}
