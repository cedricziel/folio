//! Generates web assets from the design sources in `design/`.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const HEADER: &str = "/* Generated from design/tokens.json by `make tokens`. Do not edit. */\n";

/// Token groups whose entries are `{ name, value }`, where value is a string
/// or a `{ light, dark }` pair.
const GROUPS: [&[&str]; 4] = [
    &["color", "tokens"],
    &["spacing", "tokens"],
    &["radius", "tokens"],
    &["shadow", "tokens"],
];

/// Repository root (the parent of this crate's directory).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repository root")
        .to_path_buf()
}

/// Writes the generated files, or with `check` fails when they have drifted.
pub fn run(root: &Path, check: bool) -> Result<()> {
    let tokens = std::fs::read_to_string(root.join("design/tokens.json"))
        .context("reading design/tokens.json")?;
    let components = std::fs::read_to_string(root.join("design/components.css"))
        .context("reading design/components.css")?;
    let targets = [
        ("static/tokens.css", render_tokens_css(&tokens)?),
        ("static/components.css", components),
    ];

    let mut drifted = Vec::new();
    for (rel, content) in targets {
        let path = root.join(rel);
        if check {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                drifted.push(rel);
            }
        } else {
            std::fs::write(&path, content).with_context(|| format!("writing {rel}"))?;
        }
    }
    if !drifted.is_empty() {
        bail!(
            "{} out of date with design/; run `make tokens`",
            drifted.join(", ")
        );
    }
    Ok(())
}

fn declarations(tokens: &[(String, String)], indent: &str) -> String {
    tokens.iter().fold(String::new(), |mut s, (k, v)| {
        let _ = writeln!(s, "{indent}--{k}: {v};");
        s
    })
}

fn resolve_alias(value: &str) -> String {
    match value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        Some(name) => format!("var(--{name})"),
        None => value.to_string(),
    }
}

pub fn render_tokens_css(json: &str) -> Result<String> {
    let doc: Value = serde_json::from_str(json).context("parsing tokens.json")?;

    let mut light: Vec<(String, String)> = Vec::new();
    let mut dark: Vec<(String, String)> = Vec::new();

    for path in GROUPS {
        let list = path
            .iter()
            .try_fold(&doc, |v, k| v.get(*k))
            .and_then(Value::as_array)
            .with_context(|| format!("missing {}", path.join(".")))?;
        for token in list {
            let name = token["name"].as_str().context("token without name")?;
            match &token["value"] {
                Value::String(s) => light.push((name.to_string(), resolve_alias(s))),
                Value::Object(pair) => {
                    let theme = |theme: &str| {
                        pair.get(theme)
                            .and_then(Value::as_str)
                            .map(resolve_alias)
                            .with_context(|| format!("token {name} has no {theme} value"))
                    };
                    light.push((name.to_string(), theme("light")?));
                    dark.push((name.to_string(), theme("dark")?));
                }
                _ => bail!("token {name} has an unsupported value"),
            }
        }
    }

    for family in ["sans", "mono"] {
        let value = doc["type"]["families"][family]
            .as_str()
            .with_context(|| format!("missing type.families.{family}"))?;
        light.push((format!("font-{family}"), value.to_string()));
    }

    let mut css = String::from(HEADER);
    let _ = write!(
        css,
        ":root {{\n  color-scheme: light dark;\n{}}}\n\n",
        declarations(&light, "  ")
    );
    let _ = write!(
        css,
        "@media (prefers-color-scheme: dark) {{\n  :root:not([data-theme=\"light\"]) {{\n{}  }}\n}}\n\n",
        declarations(&dark, "    ")
    );
    let _ = write!(
        css,
        ":root[data-theme=\"dark\"] {{\n  color-scheme: dark;\n{}}}\n",
        declarations(&dark, "  ")
    );
    Ok(css)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_aliases_and_splits_themes() {
        let json = r##"{
          "color": {"tokens": [
            {"name": "archive", "value": {"light": "#111", "dark": "#222"}},
            {"name": "focus", "value": "{archive}"}
          ]},
          "spacing": {"tokens": [{"name": "space-1", "value": "4px"}]},
          "radius": {"tokens": []},
          "shadow": {"tokens": []},
          "type": {"families": {"sans": "Geist", "mono": "Geist Mono"}}
        }"##;
        let css = render_tokens_css(json).unwrap();
        assert!(css.contains("  --archive: #111;"));
        assert!(css.contains("  --focus: var(--archive);"));
        assert!(css.contains("  --space-1: 4px;"));
        assert!(css.contains("  --font-sans: Geist;"));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        assert!(css.contains(":root[data-theme=\"dark\"]"));
        assert_eq!(css.matches("--archive: #222;").count(), 2);
        assert_eq!(css.matches("--focus:").count(), 1);
    }

    #[test]
    fn committed_assets_match_design_sources() {
        run(&repo_root(), true).unwrap();
    }
}
