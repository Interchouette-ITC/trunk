//! Verifies that the asset manifest describes markup emitted by a Trunk build.

use std::{fs, process::Command};

#[rstest::rstest]
#[case::unminified("never")]
#[case::minified("always")]
fn records_generated_inline_assets_and_exact_markup(#[case] minify: &str) {
    let temp = tempfile::tempdir().expect("temporary directory");
    fs::write(
        temp.path().join("Trunk.toml"),
        format!(
            r#"[build]
target = "index.html"
dist = "dist"
public_url = "/app/"
create_nonce = true
minify = "{minify}"
"#
        ),
    )
    .expect("write Trunk config");
    fs::write(
        temp.path().join("index.html"),
        r#"<!doctype html>
<html><head>
<link data-trunk rel="css" href="style.css" media="screen" />
<link data-trunk rel="inline" href="inline.css" type="css" />
</head><body>
<script data-trunk src="app.js" type="module" defer></script>
<link data-trunk rel="inline" href="inline.js" type="module" />
<link data-trunk rel="inline" href="inline.html" type="html" />
<link data-trunk rel="inline" href="inline.svg" type="svg" />
</body></html>"#,
    )
    .expect("write Trunk source HTML");
    fs::write(temp.path().join("style.css"), "body { color: red; }\n").expect("write stylesheet");
    fs::write(temp.path().join("inline.css"), "main { display: block; }\n")
        .expect("write inline stylesheet");
    fs::write(temp.path().join("app.js"), "export default 1;\n").expect("write script");
    fs::write(
        temp.path().join("inline.js"),
        "window.inlineReady = true;\n",
    )
    .expect("write inline module");
    fs::write(temp.path().join("inline.html"), "<span>inline HTML</span>")
        .expect("write inline HTML");
    fs::write(
        temp.path().join("inline.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>"#,
    )
    .expect("write inline SVG");

    let output = Command::new(env!("CARGO_BIN_EXE_trunk"))
        .arg("build")
        .current_dir(temp.path())
        .output()
        .expect("run Trunk build");
    assert!(
        output.status.success(),
        "Trunk build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let dist = temp.path().join("dist");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(dist.join("trunk-assets.json")).expect("read asset manifest"),
    )
    .expect("parse asset manifest");
    let html = fs::read_to_string(dist.join("index.html")).expect("read generated HTML");
    assert_eq!(manifest["version"], 3);

    let assets = manifest["assets"].as_array().expect("external assets");
    assert_eq!(assets.len(), 2);
    assert!(assets[0]["url"].as_str().unwrap().starts_with("/app/"));
    assert_eq!(assets[0]["role"], "stylesheet");
    assert_eq!(assets[0]["attributes"]["media"], "screen");
    assert_eq!(assets[1]["role"], "script");

    let document_assets = manifest["document_assets"]
        .as_array()
        .expect("generated document assets");
    assert_eq!(document_assets.len(), 6);
    assert_eq!(
        document_assets
            .iter()
            .map(|asset| asset["order"].as_u64().expect("asset order"))
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5]
    );

    for asset in document_assets {
        let markup = asset["html"].as_str().expect("asset markup");
        assert!(
            html.contains(markup),
            "manifest markup not present in generated HTML: {markup}\n{html}"
        );
    }

    let inline_style = document_assets[1]["html"]
        .as_str()
        .expect("inline stylesheet markup");
    assert!(inline_style.starts_with("<style"));
    assert!(inline_style.contains("nonce="));
    assert_eq!(document_assets[1]["location"], "head");

    let inline_script = document_assets[3]["html"]
        .as_str()
        .expect("inline module markup");
    assert!(inline_script.starts_with("<script"));
    assert!(inline_script.contains("type=module") || inline_script.contains("type=\"module\""));
    assert!(inline_script.contains("nonce="));
    assert_eq!(document_assets[3]["location"], "body");
    assert_eq!(document_assets[4]["html"], "<span>inline HTML</span>");
    assert_eq!(document_assets[4]["location"], "body");
    let inline_svg = document_assets[5]["html"]
        .as_str()
        .expect("inline SVG markup");
    assert!(inline_svg.starts_with("<svg"));
    assert!(inline_svg.contains("<circle"));
    assert_eq!(document_assets[5]["location"], "body");
}
