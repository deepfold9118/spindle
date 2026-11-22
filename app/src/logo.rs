//! The Spindle mark, drawn from the logo's SVG path (assets/spindle-logo.svg).
//! The program icon (window, taskbar and the Windows .exe via build.rs) and the
//! title-bar mark are both rasterised from this one path, so they always match.
//! Self-contained (no egui) because build.rs includes this file too.

#![allow(dead_code)]

/// Logo colour (the theme accent).
pub const COLOR: [u8; 3] = [0xfb, 0x9e, 0x35];

/// The logo outline: absolute M/C/L/Z commands from the SVG, filled even-odd
/// (the outer wheel minus the two blade slots and the hub hole).
pub const PATH: &str = include_str!("../assets/spindle-logo.path");

/// Flatten the path into closed polygons (cubic segments split into straight pieces).
pub fn polygons() -> Vec<Vec<[f32; 2]>> {
    const STEPS: usize = 12;
    let mut nums: Vec<f32> = Vec::new();
    let mut cmds: Vec<(char, Vec<f32>)> = Vec::new();
    let mut cur_cmd: Option<char> = None;
    let mut num = String::new();
    let flush = |num: &mut String, nums: &mut Vec<f32>| {
        if !num.is_empty() {
            nums.push(num.parse().unwrap_or(0.0));
            num.clear();
        }
    };
    for ch in PATH.chars() {
        match ch {
            'M' | 'C' | 'L' | 'Z' | 'z' => {
                flush(&mut num, &mut nums);
                if let Some(c) = cur_cmd.take() {
                    cmds.push((c, std::mem::take(&mut nums)));
                }
                cur_cmd = Some(ch.to_ascii_uppercase());
            }
            '-' => {
                // A minus starts a new number unless it follows an exponent.
                if !num.ends_with('e') {
                    flush(&mut num, &mut nums);
                }
                num.push(ch);
            }
            '0'..='9' | '.' | 'e' => num.push(ch),
            _ => flush(&mut num, &mut nums),
        }
    }
    flush(&mut num, &mut nums);
    if let Some(c) = cur_cmd {
        cmds.push((c, nums));
    }

    let mut polys = Vec::new();
    let mut poly: Vec<[f32; 2]> = Vec::new();
    let mut at = [0.0f32; 2];
    for (c, n) in cmds {
        match c {
            'M' => {
                if poly.len() > 2 {
                    polys.push(std::mem::take(&mut poly));
                }
                poly.clear();
                at = [n[0], n[1]];
                poly.push(at);
                for p in n[2..].chunks_exact(2) {
                    at = [p[0], p[1]];
                    poly.push(at);
                }
            }
            'L' => {
                for p in n.chunks_exact(2) {
                    at = [p[0], p[1]];
                    poly.push(at);
                }
            }
            'C' => {
                for s in n.chunks_exact(6) {
                    let (p0, p1, p2, p3) = (at, [s[0], s[1]], [s[2], s[3]], [s[4], s[5]]);
                    for i in 1..=STEPS {
                        let t = i as f32 / STEPS as f32;
                        let u = 1.0 - t;
                        let (a, b, c2, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        poly.push([
                            a * p0[0] + b * p1[0] + c2 * p2[0] + d * p3[0],
                            a * p0[1] + b * p1[1] + c2 * p2[1] + d * p3[1],
                        ]);
                    }
                    at = p3;
                }
            }
            'Z' => {
                if poly.len() > 2 {
                    polys.push(std::mem::take(&mut poly));
                }
                poly.clear();
            }
            _ => {}
        }
    }
    if poly.len() > 2 {
        polys.push(poly);
    }
    polys
}

/// Coverage (0..1) of the mark centred in a `size`×`size` square, aspect preserved.
/// Scanline fill, even-odd, 4 sub-rows per pixel with exact horizontal coverage.
pub fn coverage(size: u32) -> Vec<f32> {
    let polys = polygons();
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in polys.iter().flatten() {
        minx = minx.min(p[0]);
        miny = miny.min(p[1]);
        maxx = maxx.max(p[0]);
        maxy = maxy.max(p[1]);
    }
    let n = size as f32;
    let k = n * 0.94 / (maxx - minx).max(maxy - miny);
    let ox = (n - (maxx - minx) * k) / 2.0 - minx * k;
    let oy = (n - (maxy - miny) * k) / 2.0 - miny * k;
    let edges: Vec<([f32; 2], [f32; 2])> = polys
        .iter()
        .flat_map(|poly| {
            (0..poly.len()).map(move |i| {
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                ([a[0] * k + ox, a[1] * k + oy], [b[0] * k + ox, b[1] * k + oy])
            })
        })
        .collect();

    const SUB: u32 = 4;
    let mut cov = vec![0.0f32; (size * size) as usize];
    let mut xs: Vec<f32> = Vec::new();
    for py in 0..size {
        let row = &mut cov[(py * size) as usize..((py + 1) * size) as usize];
        for s in 0..SUB {
            let y = py as f32 + (s as f32 + 0.5) / SUB as f32;
            xs.clear();
            for (a, b) in &edges {
                if (a[1] <= y && y < b[1]) || (b[1] <= y && y < a[1]) {
                    xs.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            xs.sort_by(|p, q| p.total_cmp(q));
            for span in xs.chunks_exact(2) {
                let (x0, x1) = (span[0].max(0.0), span[1].min(n));
                if x1 <= x0 {
                    continue;
                }
                let (first, last) = (x0.floor() as usize, (x1.ceil() as usize).min(size as usize));
                for (px, c) in row.iter_mut().enumerate().take(last).skip(first) {
                    let overlap = (x1.min(px as f32 + 1.0) - x0.max(px as f32)).max(0.0);
                    *c += overlap / SUB as f32;
                }
            }
        }
    }
    cov
}

/// RGBA pixels (straight alpha) of the mark in its logo colour.
pub fn rasterize(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for c in coverage(size) {
        out.extend_from_slice(&[COLOR[0], COLOR[1], COLOR[2], (c.clamp(0.0, 1.0) * 255.0).round() as u8]);
    }
    out
}

/// White RGBA mask of the mark, for tinting with the theme accent in the UI.
pub fn rasterize_mask(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for c in coverage(size) {
        out.extend_from_slice(&[255, 255, 255, (c.clamp(0.0, 1.0) * 255.0).round() as u8]);
    }
    out
}
