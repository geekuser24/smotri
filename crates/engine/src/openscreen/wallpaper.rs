//! Resolve OpenScreen's bundled wallpaper URLs without shipping its artwork.
use super::{Result, Session, embedded, error, read};
use base64::Engine as _;
use std::path::{Path, PathBuf};

pub(super) fn resolve(s: &Session, project: &str, value: &str) -> Result<String> {
    let direct = super::resolve(project, value);
    if !value.starts_with("/wallpapers/") {
        return Ok(direct);
    }
    let name = value.strip_prefix("/wallpapers/").ok_or_else(|| error("invalid wallpaper path"))?;
    if name.is_empty() || name.contains(['/', '\\']) || matches!(name, "." | "..") {
        return Err(error("invalid bundled wallpaper name"));
    }
    let mime = match Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "jpeg",
        Some("png") => "png",
        Some("webp") => "webp",
        _ => return Err(error("bundled wallpaper must be JPEG, PNG or WebP")),
    };
    if s.services.file_size(&direct).is_ok() {
        return Ok(direct);
    }
    let mut roots = vec![Path::new(project).parent().unwrap_or(Path::new(".")).join("wallpapers")];
    for app in ["Openscreen.app", "OpenScreen.app"] {
        roots.push(Path::new("/Applications").join(app).join("Contents/Resources/wallpapers"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications").join(app).join("Contents/Resources/wallpapers"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for app in ["openscreen", "OpenScreen"] {
            roots.push(PathBuf::from(&local).join("Programs").join(app).join("resources/wallpapers"));
        }
    }
    for base in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(base) {
            roots.push(PathBuf::from(root).join("OpenScreen/resources/wallpapers"));
        }
    }
    for root in ["/opt/openscreen/resources/wallpapers", "/usr/lib/openscreen/resources/wallpapers"] {
        roots.push(PathBuf::from(root));
    }
    for root in roots {
        let file = root.join(name).to_string_lossy().into_owned();
        if s.services.file_size(&file).is_ok() {
            let bytes = read(s, &file, 16 * 1024 * 1024)?;
            let url = format!("data:image/{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
            return embedded(s, project, "background", &url);
        }
    }
    Err(error(format!("bundled wallpaper {name} was not found; keep OpenScreen installed or copy the image into a wallpapers folder beside the project")))
}
