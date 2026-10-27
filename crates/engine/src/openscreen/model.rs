//! OpenScreen v2 JSON, bounded and validated before project edits.
use super::{Result, error};
use serde::Deserialize;
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Point {
    pub cx: f64,
    pub cy: f64,
}
impl Default for Point {
    fn default() -> Self {
        Self { cx: 0.5, cy: 0.5 }
    }
}
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for Rect {
    fn default() -> Self {
        Self { x: 0., y: 0., width: 1., height: 1. }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pose {
    pub time_ms: f64,
    pub layout_preset: String,
    pub screen: Option<Rect>,
    pub webcam: Option<Rect>,
    #[serde(default = "smooth")]
    pub easing: String,
}
fn smooth() -> String {
    "smooth".into()
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Zoom {
    pub start_ms: f64,
    pub end_ms: f64,
    pub depth: u32,
    pub focus: Point,
    #[serde(default)]
    pub focus_mode: String,
    pub custom_scale: Option<f64>,
    #[serde(default, rename = "rotation3D")]
    pub rotation3d: serde_json::Value,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub start_ms: f64,
    pub end_ms: f64,
    pub speed: Option<f64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub screen_video_path: String,
    pub webcam_video_path: Option<String>,
    #[serde(default)]
    pub cursor_capture_mode: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Document {
    pub version: u32,
    pub media: Media,
    pub editor: Editor,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Editor {
    pub wallpaper: String,
    pub padding: f64,
    pub border_radius: f64,
    pub shadow_intensity: f64,
    pub motion_blur_amount: f64,
    pub crop_region: Rect,
    pub aspect_ratio: String,
    pub webcam_layout_preset: String,
    pub webcam_mask_shape: String,
    pub webcam_size_preset: f64,
    pub webcam_position: Option<Point>,
    pub webcam_mirrored: bool,
    pub webcam_offset_ms: f64,
    pub webcam_reactive_zoom: bool,
    pub composition_keyframes: Vec<Pose>,
    pub zoom_regions: Vec<Zoom>,
    pub trim_regions: Vec<Region>,
    pub speed_regions: Vec<Region>,
    pub annotation_regions: Vec<serde_json::Value>,
    pub cursor_size: f64,
    pub cursor_smoothing: f64,
    pub cursor_click_bounce: f64,
    pub show_cursor: bool,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            wallpaper: String::new(),
            padding: 0.,
            border_radius: 0.,
            shadow_intensity: 0.,
            motion_blur_amount: 0.,
            crop_region: Rect::default(),
            aspect_ratio: "16:9".into(),
            webcam_layout_preset: "picture-in-picture".into(),
            webcam_mask_shape: "rectangle".into(),
            webcam_size_preset: 25.,
            webcam_position: None,
            webcam_mirrored: false,
            webcam_offset_ms: 0.,
            webcam_reactive_zoom: false,
            composition_keyframes: vec![],
            zoom_regions: vec![],
            trim_regions: vec![],
            speed_regions: vec![],
            annotation_regions: vec![],
            cursor_size: 3.,
            cursor_smoothing: 0.67,
            cursor_click_bounce: 2.5,
            show_cursor: true,
        }
    }
}
#[derive(Debug, Deserialize, Default)]
pub struct Cursor {
    #[serde(default)]
    pub samples: Vec<Sample>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub time_ms: f64,
    pub cx: f64,
    pub cy: f64,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub interaction_type: String,
    pub asset_id: String,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub image_data_url: String,
    pub width: f64,
    pub height: f64,
    pub hotspot_x: f64,
    pub hotspot_y: f64,
    #[serde(default = "one")]
    pub scale_factor: f64,
}
fn one() -> f64 {
    1.
}
pub fn bounded(v: f64, min: f64, max: f64, name: &str) -> Result<()> {
    if !v.is_finite() || v < min || v > max { Err(error(format!("invalid {name}"))) } else { Ok(()) }
}
fn rect(r: Rect) -> Result<()> {
    for v in [r.x, r.y] {
        bounded(v, -3., 3., "layer position")?;
    }
    for v in [r.width, r.height] {
        bounded(v, 0.001, 3., "layer size")?;
    }
    Ok(())
}
fn point(p: Point) -> Result<()> {
    bounded(p.cx, -3., 3., "focus x")?;
    bounded(p.cy, -3., 3., "focus y")
}
fn preset(p: &str) -> Result<()> {
    if matches!(p, "picture-in-picture" | "dual-frame" | "vertical-stack" | "no-webcam") { Ok(()) } else { Err(error(format!("unsupported layout: {p}"))) }
}
pub fn parse(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(error("project exceeds 128 MiB"));
    }
    let mut d: Document = serde_json::from_slice(bytes).map_err(|e| error(format!("invalid project JSON: {e}")))?;
    if d.version != 2 {
        return Err(error(format!("unsupported project version {}", d.version)));
    }
    let e = &mut d.editor;
    if e.wallpaper.starts_with('#') && !matches!(e.wallpaper.to_ascii_lowercase().as_str(), "#000" | "#000000") {
        return Err(error("solid-color wallpaper is not yet supported; use an image wallpaper"));
    }
    if e.composition_keyframes.len() > 2000 || e.zoom_regions.len() > 2000 || e.trim_regions.len() > 2000 || e.speed_regions.len() > 2000 {
        return Err(error("too many regions"));
    }
    rect(e.crop_region)?;
    if e.crop_region.x < 0. || e.crop_region.y < 0. || e.crop_region.x + e.crop_region.width > 1.00001 || e.crop_region.y + e.crop_region.height > 1.00001 {
        return Err(error("invalid source crop"));
    }
    preset(&e.webcam_layout_preset)?;
    bounded(e.padding, 0., 100., "padding")?;
    bounded(e.webcam_size_preset, 2., 100., "webcam size")?;
    bounded(e.webcam_offset_ms, -10000., 10000., "camera offset")?;
    for (v, n) in [
        (e.border_radius, "border radius"),
        (e.shadow_intensity, "shadow"),
        (e.motion_blur_amount, "motion blur"),
        (e.cursor_size, "cursor size"),
        (e.cursor_smoothing, "cursor smoothing"),
        (e.cursor_click_bounce, "cursor bounce"),
    ] {
        bounded(v, 0., 1000., n)?;
    }
    if let Some(p) = e.webcam_position {
        point(p)?;
    }
    for p in &e.composition_keyframes {
        bounded(p.time_ms, 0., 7_200_000., "composition time")?;
        preset(&p.layout_preset)?;
        if let Some(r) = p.screen {
            rect(r)?;
        }
        if let Some(r) = p.webcam {
            rect(r)?;
        }
        if !matches!(p.easing.as_str(), "smooth" | "linear" | "hold") {
            return Err(error("unsupported composition easing"));
        }
    }
    e.composition_keyframes.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    e.composition_keyframes.dedup_by(|a, b| {
        if a.time_ms == b.time_ms {
            *b = a.clone();
            true
        } else {
            false
        }
    });
    for z in &e.zoom_regions {
        bounded(z.start_ms, 0., 7_200_000., "zoom start")?;
        bounded(z.end_ms, z.start_ms + 0.001, 7_200_000., "zoom end")?;
        point(z.focus)?;
        if !(1..=6).contains(&z.depth) {
            return Err(error("invalid zoom depth"));
        }
        if let Some(v) = z.custom_scale {
            bounded(v, 1., 5., "zoom scale")?;
        }
    }
    e.zoom_regions.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    for r in e.trim_regions.iter().chain(&e.speed_regions) {
        bounded(r.start_ms, 0., 7_200_000., "region start")?;
        bounded(r.end_ms, r.start_ms + 0.001, 7_200_000., "region end")?;
        if let Some(v) = r.speed {
            bounded(v, 0.1, 10., "speed")?;
        }
    }
    if !e.annotation_regions.is_empty() {
        return Err(error("annotations are not yet supported; remove them in a copy of the OpenScreen project before importing"));
    }
    Ok(d)
}
pub fn cursor(bytes: &[u8]) -> Result<Cursor> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(error("cursor recording exceeds 32 MiB"));
    }
    let mut c: Cursor = serde_json::from_slice(bytes).map_err(|e| error(format!("invalid cursor JSON: {e}")))?;
    if c.samples.len() > 250_000 || c.assets.len() > 128 {
        return Err(error("cursor recording is too large"));
    }
    for s in &c.samples {
        bounded(s.time_ms, 0., 7_200_000., "cursor time")?;
        point(Point { cx: s.cx, cy: s.cy })?;
    }
    for a in &c.assets {
        bounded(a.scale_factor, 0.1, 8., "cursor scale factor")?;
        bounded(a.width, 1., 1024., "cursor width")?;
        bounded(a.height, 1., 1024., "cursor height")?;
        bounded(a.hotspot_x, 0., a.width, "cursor hotspot")?;
        bounded(a.hotspot_y, 0., a.height, "cursor hotspot")?;
    }
    c.samples.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    Ok(c)
}
