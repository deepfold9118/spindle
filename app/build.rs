//! Windows build steps: embed the program icon (made from the same spindle mark as
//! the title bar) and, if a running copy locks the .exe, move it aside so the linker
//! can write the new one (the open window keeps working).

#[path = "src/logo.rs"]
mod logo;

fn png_bytes(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&logo::rasterize(size)).expect("png data");
    }
    out
}

/// Write a multi-size .ico (PNG entries) and compile it into the program.
fn embed_icon() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png_bytes(s)).collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[b, b, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    let ico_path = out_dir.join("spindle_app.ico");
    std::fs::write(&ico_path, ico).unwrap();
    let rc_path = out_dir.join("spindle_app.rc");
    let ico_str = ico_path.display().to_string().replace(char::from(92), "/");
    std::fs::write(&rc_path, format!("1 ICON \"{ico_str}\"{}", "\n")).unwrap();
    let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=assets/spindle-logo.path");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_icon();
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    let Some(profile_dir) = std::path::Path::new(&out).ancestors().nth(3) else { return };
    let exe = profile_dir.join("spindle_app.exe");
    if let Ok(rd) = std::fs::read_dir(profile_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("spindle_app.old-") && name.ends_with(".exe") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    if exe.exists() && std::fs::OpenOptions::new().write(true).open(&exe).is_err() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = std::fs::rename(&exe, profile_dir.join(format!("spindle_app.old-{stamp}.exe")));
    }
}
