//! Web app manifest and icons, drawn from the folder-tab mark.

use anyhow::{Context, Result};
use image::{ImageEncoder, Rgba, RgbaImage};
use serde_json::{json, Value};

/// A rounded rectangle in unit coordinates.
struct Shape {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    r: f32,
}

/// Tab, then body. Together they read as a folder.
const MARK: [Shape; 2] = [
    Shape {
        x0: 0.19,
        y0: 0.24,
        x1: 0.47,
        y1: 0.50,
        r: 0.05,
    },
    Shape {
        x0: 0.19,
        y0: 0.38,
        x1: 0.81,
        y1: 0.80,
        r: 0.06,
    },
];

/// Corner radius of the "any" icon background, as a fraction of its size.
const BACKGROUND_RADIUS: f32 = 0.22;

struct Palette {
    paper: [u8; 3],
    archive: [u8; 3],
    paper_hex: String,
    archive_hex: String,
}

fn light_color(tokens: &Value, name: &str) -> Result<String> {
    tokens["color"]["tokens"]
        .as_array()
        .and_then(|list| list.iter().find(|t| t["name"] == name))
        .and_then(|t| t["value"]["light"].as_str())
        .map(str::to_string)
        .with_context(|| format!("no light color token {name}"))
}

fn parse_hex(hex: &str) -> Result<[u8; 3]> {
    let h = hex.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).context("bad hex color");
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

fn palette(tokens: &Value) -> Result<Palette> {
    let paper_hex = light_color(tokens, "paper")?;
    let archive_hex = light_color(tokens, "archive")?;
    Ok(Palette {
        paper: parse_hex(&paper_hex)?,
        archive: parse_hex(&archive_hex)?,
        paper_hex,
        archive_hex,
    })
}

pub struct Icon {
    pub file: &'static str,
    pub size: u32,
    /// Mark scale; maskable icons keep the mark inside the 80% safe zone.
    scale: f32,
    /// Corner radius of the background; zero fills the whole canvas.
    radius: f32,
    pub purpose: &'static str,
}

pub const ICONS: [Icon; 4] = [
    Icon {
        file: "icon-192.png",
        size: 192,
        scale: 1.0,
        radius: BACKGROUND_RADIUS,
        purpose: "any",
    },
    Icon {
        file: "icon-512.png",
        size: 512,
        scale: 1.0,
        radius: BACKGROUND_RADIUS,
        purpose: "any",
    },
    Icon {
        file: "icon-maskable-512.png",
        size: 512,
        scale: 0.8,
        radius: 0.0,
        purpose: "maskable",
    },
    Icon {
        file: "apple-touch-icon.png",
        size: 180,
        scale: 1.0,
        radius: 0.0,
        purpose: "",
    },
];

fn inside_rounded_rect(x: f32, y: f32, s: &Shape) -> bool {
    if x < s.x0 || x > s.x1 || y < s.y0 || y > s.y1 {
        return false;
    }
    let cx = x.clamp(s.x0 + s.r, s.x1 - s.r);
    let cy = y.clamp(s.y0 + s.r, s.y1 - s.r);
    (x - cx).powi(2) + (y - cy).powi(2) <= s.r * s.r
}

fn inside_background(x: f32, y: f32, radius: f32) -> bool {
    inside_rounded_rect(
        x,
        y,
        &Shape {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
            r: radius,
        },
    )
}

fn render(icon: &Icon, colors: &Palette) -> RgbaImage {
    const SAMPLES: u32 = 4;
    let n = icon.size;
    let mut img = RgbaImage::new(n, n);
    for py in 0..n {
        for px in 0..n {
            let (mut bg, mut mark) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / n as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / n as f32;
                    if inside_background(x, y, icon.radius) {
                        bg += 1;
                    }
                    let mx = (x - 0.5) / icon.scale + 0.5;
                    let my = (y - 0.5) / icon.scale + 0.5;
                    if MARK.iter().any(|s| inside_rounded_rect(mx, my, s)) {
                        mark += 1;
                    }
                }
            }
            let total = (SAMPLES * SAMPLES) as f32;
            let mark_a = mark as f32 / total;
            let bg_a = bg as f32 / total;
            let mix = |i: usize| {
                (colors.archive[i] as f32 * mark_a + colors.paper[i] as f32 * (1.0 - mark_a))
                    .round() as u8
            };
            let alpha = (bg_a.max(mark_a) * 255.0).round() as u8;
            img.put_pixel(px, py, Rgba([mix(0), mix(1), mix(2), alpha]));
        }
    }
    img
}

fn png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .context("encoding png")?;
    Ok(out)
}

/// `(path relative to static/, bytes)` for every icon.
pub fn icons(tokens: &Value) -> Result<Vec<(String, Vec<u8>)>> {
    let colors = palette(tokens)?;
    let mut out = Vec::new();
    for icon in &ICONS {
        out.push((format!("icons/{}", icon.file), png(&render(icon, &colors))?));
    }
    out.push(("icons/icon.svg".to_string(), svg(&colors).into_bytes()));
    Ok(out)
}

fn svg(colors: &Palette) -> String {
    let rect = |s: &Shape| {
        format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{:.1}" fill="{}"/>"#,
            s.x0 * 512.0,
            s.y0 * 512.0,
            (s.x1 - s.x0) * 512.0,
            (s.y1 - s.y0) * 512.0,
            s.r * 512.0,
            colors.archive_hex
        )
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 512 512\">\n  <rect width=\"512\" height=\"512\" rx=\"{:.1}\" fill=\"{}\"/>\n  {}\n  {}\n</svg>\n",
        BACKGROUND_RADIUS * 512.0,
        colors.paper_hex,
        rect(&MARK[0]),
        rect(&MARK[1])
    )
}

pub fn manifest(tokens: &Value) -> Result<String> {
    let colors = palette(tokens)?;
    let mut icons: Vec<Value> = ICONS
        .iter()
        .filter(|i| !i.purpose.is_empty())
        .map(|i| {
            json!({
                "src": format!("/static/icons/{}", i.file),
                "sizes": format!("{0}x{0}", i.size),
                "type": "image/png",
                "purpose": i.purpose,
            })
        })
        .collect();
    icons.push(json!({
        "src": "/static/icons/icon.svg",
        "sizes": "any",
        "type": "image/svg+xml",
        "purpose": "any",
    }));

    let manifest = json!({
        "name": "Folio",
        "short_name": "Folio",
        "description": "Keep letters, invoices and contracts filed and findable.",
        "id": "/",
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "background_color": colors.paper_hex,
        "theme_color": colors.paper_hex,
        "icons": icons,
        "share_target": {
            "action": "/share",
            "method": "POST",
            "enctype": "multipart/form-data",
            "params": {
                "title": "title",
                "files": [{ "name": "file", "accept": ["image/*", "application/pdf"] }]
            }
        }
    });
    Ok(serde_json::to_string_pretty(&manifest)? + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(crate::repo_root().join("design/tokens.json")).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn manifest_takes_colors_from_tokens_and_declares_share_target() {
        let m: Value = serde_json::from_str(&manifest(&tokens()).unwrap()).unwrap();
        assert_eq!(m["name"], "Folio");
        assert_eq!(m["theme_color"], "#f6f6f2");
        assert_eq!(m["share_target"]["action"], "/share");
        assert_eq!(m["share_target"]["params"]["files"][0]["name"], "file");
        let purposes: Vec<_> = m["icons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["purpose"].as_str().unwrap())
            .collect();
        assert!(purposes.contains(&"maskable"));
    }

    #[test]
    fn icons_have_the_declared_sizes_and_draw_the_mark() {
        let out = icons(&tokens()).unwrap();
        for icon in &ICONS {
            let (_, bytes) = out
                .iter()
                .find(|(p, _)| p.ends_with(icon.file))
                .unwrap_or_else(|| panic!("{} generated", icon.file));
            let img = image::load_from_memory(bytes).unwrap().to_rgba8();
            assert_eq!((img.width(), img.height()), (icon.size, icon.size));
            let centre = img.get_pixel(icon.size / 2, icon.size * 6 / 10);
            assert_eq!(
                centre.0,
                [0x1f, 0x6b, 0x5c, 255],
                "{} body is archive green",
                icon.file
            );
        }
    }

    #[test]
    fn maskable_icon_is_full_bleed_and_any_icon_has_round_corners() {
        let out = icons(&tokens()).unwrap();
        let load = |name: &str| {
            let (_, bytes) = out.iter().find(|(p, _)| p.ends_with(name)).unwrap();
            image::load_from_memory(bytes).unwrap().to_rgba8()
        };
        assert_eq!(load("icon-maskable-512.png").get_pixel(0, 0)[3], 255);
        assert_eq!(load("icon-512.png").get_pixel(0, 0)[3], 0);
    }
}
