use super::super::trunk_id_selector;
use crate::{
    common::{html_rewrite::Document, nonce_attr},
    config::{rt::RtcBuild, types::CrossOrigin},
    pipelines::rust::{RustAppType, sri::SriBuilder, wasm_bindgen::WasmBindgenFeatures},
};
use anyhow::bail;
use std::{collections::HashMap, sync::Arc};

/// The output of a cargo build pipeline.
pub struct RustAppOutput {
    /// The runtime build config.
    pub cfg: Arc<RtcBuild>,
    /// The ID of this pipeline.
    pub id: Option<usize>,
    /// The filename of the generated JS loader file written to the dist dir.
    pub js_output: String,
    /// The filename of the generated WASM file written to the dist dir.
    pub wasm_output: String,
    /// The size of the WASM file
    pub wasm_size: u64,
    /// Is this module main or a worker.
    pub r#type: RustAppType,
    /// The cross-origin setting for loading the resources
    pub cross_origin: CrossOrigin,
    /// The output digests for the sub-resources
    pub integrities: SriBuilder,
    /// Import functions exported from Rust into JavaScript
    pub import_bindings: bool,
    /// The name of the WASM bindings import
    pub import_bindings_name: Option<String>,
    /// Compression algorithm used for the WASM file
    pub compression_algorithm: Option<String>,
    /// The target of the initializer module
    pub initializer: Option<String>,
    /// The features supported by the version of wasm-bindgen used
    pub wasm_bindgen_features: WasmBindgenFeatures,
}

pub fn pattern_evaluate(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (k, v) in params.iter() {
        let pattern = format!("{{{}}}", k.as_str());
        if let Some(file_path) = v.strip_prefix('@') {
            if let Ok(contents) = std::fs::read_to_string(file_path) {
                result = str::replace(result.as_str(), &pattern, contents.as_str());
            }
        } else {
            result = str::replace(result.as_str(), &pattern, v);
        }
    }
    result
}

impl RustAppOutput {
    pub async fn finalize(self, dom: &mut Document) -> anyhow::Result<()> {
        if self.r#type == RustAppType::Worker {
            // Skip the script tag and preload links for workers, and remove the link tag only.
            // Workers are initialized and managed by the app itself at runtime.
            if let Some(id) = self.id {
                dom.remove(&trunk_id_selector(id))?;
            }
            return Ok(());
        }

        if !self.cfg.inject_scripts {
            // Configuration directed we do not inject any scripts.
            return Ok(());
        }

        let (base, js, wasm, head, body) = (
            &self.cfg.public_url,
            &self.js_output,
            &self.wasm_output,
            "html head",
            "html body",
        );
        let (pattern_script, pattern_preload) =
            (&self.cfg.pattern_script, &self.cfg.pattern_preload);
        let mut params = self.cfg.pattern_params.clone();
        params.insert("base".to_owned(), base.to_string());
        params.insert("js".to_owned(), js.clone());
        params.insert("wasm".to_owned(), wasm.clone());
        params.insert("crossorigin".to_owned(), self.cross_origin.to_string());

        if let Some(pattern) = pattern_preload {
            dom.append_html(head, &pattern_evaluate(pattern, &params))?;
        } else {
            self.integrities.clone().build().inject(
                dom,
                head,
                base,
                self.cross_origin,
                &self.cfg.create_nonce,
            )?;
        }

        let script = match pattern_script {
            Some(pattern) => pattern_evaluate(pattern, &params),
            None => self.default_initializer(base, js, wasm),
        };

        match self.id {
            Some(id) => dom.replace_with_html(&trunk_id_selector(id), &script)?,
            None => {
                if dom.len(body)? == 0 {
                    bail!(
                        r#"Document has neither a <link data-trunk rel="rust"/> nor a <body>. Either one must be present."#
                    );
                }
                dom.append_html(body, &script)?
            }
        }

        Ok(())
    }

    /// create the default initializer script section
    fn default_initializer(&self, base: &str, js: &str, wasm: &str) -> String {
        let (import, bind) = match self.import_bindings {
            true => (
                ", * as bindings",
                format!(
                    r#"
window.{bindings} = bindings;
"#,
                    bindings = self
                        .import_bindings_name
                        .as_deref()
                        .unwrap_or("wasmBindings")
                ),
            ),
            false => ("", String::new()),
        };

        let nonce = nonce_attr(&self.cfg.create_nonce);

        // the code to fire the `TrunkApplicationStarted` event
        let fire = r#"
dispatchEvent(new CustomEvent("TrunkApplicationStarted", {detail: {wasm}}));
"#;

        let compression = if let Some(algorithm) = &self.compression_algorithm {
            format!(
                r#"
const resp = await fetch('{base}{wasm}');
if (!resp.ok) {{
    throw new Error('Failed to fetch WASM module: ' + resp.statusText);
}}

const decompressStream = resp.body.pipeThrough(new DecompressionStream('{algorithm}'));
const wasmBytes = await new Response(decompressStream).arrayBuffer();
                "#
            )
        } else {
            String::new()
        };

        let init_with_object = self.wasm_bindgen_features.init_with_object;

        match &self.initializer {
            None => {
                format!(
                    r#"
<script type="module"{nonce}>
import init{import} from '{base}{js}';
{compression}
const wasm = await init({init_arg});

{bind}
{fire}
</script>"#,
                    init_arg = {
                        let param = if self.compression_algorithm.is_some() {
                            "wasmBytes".to_string()
                        } else {
                            format!("'{base}{wasm}'")
                        };

                        if init_with_object {
                            format!("{{ module_or_path: {param} }}")
                        } else {
                            param
                        }
                    }
                )
            }
            Some(initializer) => format!(
                r#"
<script type="module"{nonce}>
{init}

import init{import} from '{base}{js}';
import initializer from '{base}{initializer}';

const wasm = await __trunkInitializer(init, '{base}{wasm}', {size}, initializer(), {init_with_object}{algorithm});

{bind}
{fire}
</script>"#,
                init = include_str!("initializer.js"),
                size = self.wasm_size,
                algorithm = if let Some(algorithm) = &self.compression_algorithm {
                    format!(", '{algorithm}'")
                } else {
                    String::new()
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pipelines::{
            TrunkAssetPipelineOutput,
            manifest::AssetManifest,
            rust::sri::{SriOptions, SriType},
        },
        processing::integrity::IntegrityType,
    };

    #[rstest::rstest]
    #[case::explicit(Some(0), true)]
    #[case::implicit(None, true)]
    #[case::missing_head(None, false)]
    #[tokio::test]
    async fn manifest_records_generated_wasm_bootstrap_and_preloads(
        #[case] id: Option<usize>,
        #[case] has_head: bool,
    ) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let mut cfg = RtcBuild::new_test(temp.path()).await.expect("test config");
        cfg.public_url = "/app/".parse().expect("public URL");
        cfg.create_nonce = Some("test-nonce".to_owned());
        let js = "app-123.js";
        let wasm = "app-123_bg.wasm";
        let wasm_bytes = b"\0asm\x01\0\0\0";
        std::fs::write(
            cfg.staging_dist.join(js),
            "export default async function init() { return {}; }",
        )
        .expect("write JavaScript output");
        std::fs::write(cfg.staging_dist.join(wasm), wasm_bytes).expect("write WASM output");

        let mut integrities = SriBuilder::new(IntegrityType::Sha384);
        for (rel, name, options) in [
            (SriType::ModulePreload, js, SriOptions::default()),
            (
                SriType::Preload,
                wasm,
                SriOptions::default()
                    .r#as("fetch")
                    .r#type("application/wasm"),
            ),
        ] {
            integrities
                .record_file(rel, name, options, cfg.staging_dist.join(name))
                .await
                .expect("record preload integrity");
        }

        let output = RustAppOutput {
            cfg: Arc::new(cfg),
            id,
            js_output: js.to_owned(),
            wasm_output: wasm.to_owned(),
            wasm_size: wasm_bytes.len() as u64,
            r#type: RustAppType::Main,
            cross_origin: CrossOrigin::Anonymous,
            integrities,
            import_bindings: true,
            import_bindings_name: Some("testBindings".to_owned()),
            compression_algorithm: None,
            initializer: None,
            wasm_bindgen_features: WasmBindgenFeatures {
                init_with_object: true,
            },
        };
        let head = if has_head {
            if id.is_some() {
                r#"<head><link data-trunk-id="0" rel="rust"></head>"#
            } else {
                "<head></head>"
            }
        } else {
            ""
        };
        let mut document = Document::new(
            format!("<html>{head}<body></body></html>"),
            Default::default(),
        )
        .expect("source document");
        let locations = [(trunk_id_selector(0), (0, "head".to_owned()))].into();
        let mut manifest = AssetManifest::new("/app/", "index.html");

        TrunkAssetPipelineOutput::RustApp(output)
            .finalize_with_manifest(&mut document, &mut manifest, &locations)
            .await
            .expect("finalize Rust pipeline and manifest");

        let html = String::from_utf8(document.into_inner()).expect("generated HTML");
        assert_eq!(manifest.wasm.len(), 1);
        assert_eq!(manifest.wasm[0].js_url, "/app/app-123.js");
        assert_eq!(manifest.wasm[0].wasm_url, "/app/app-123_bg.wasm");
        assert_eq!(manifest.document_assets.len(), if has_head { 3 } else { 1 });
        for asset in &manifest.document_assets {
            assert!(html.contains(&asset.html), "phantom asset: {}", asset.html);
            assert_eq!(asset.order, id);
            assert!(asset.html.contains(r#"nonce="test-nonce""#));
        }
        let bootstrap = manifest.document_assets.last().expect("bootstrap fragment");
        assert_eq!(
            bootstrap.location,
            if id.is_some() { "head" } else { "body" }
        );
        assert!(bootstrap.html.contains(r#"<script type="module""#));
        assert!(
            bootstrap
                .html
                .contains("import init, * as bindings from '/app/app-123.js'")
        );
        assert!(
            bootstrap
                .html
                .contains("module_or_path: '/app/app-123_bg.wasm'")
        );
        assert!(bootstrap.html.contains("window.testBindings = bindings"));
        assert!(bootstrap.html.contains("TrunkApplicationStarted"));
        assert_eq!(html.matches("<script").count(), 1);

        if has_head {
            for preload in &manifest.document_assets[..2] {
                assert_eq!(preload.location, "head");
                assert!(preload.html.contains("integrity=\"sha384-"));
                assert!(preload.html.contains("crossorigin=\"anonymous\""));
            }
            assert!(
                manifest.document_assets[0]
                    .html
                    .contains("rel=\"modulepreload\"")
            );
            assert!(
                manifest.document_assets[1]
                    .html
                    .contains("type=\"application/wasm\"")
            );
        }
        let serialized = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(
            serialized["document_assets"].as_array().unwrap().len(),
            manifest.document_assets.len()
        );
    }
}
