//! The Spindle mark, drawn from the symbol artwork (artwork/spindle_symbol.svg).
//! The program icon (window, taskbar and the Windows .exe via build.rs) and the
//! title-bar mark are both rasterised from this one path, so they always match.
//! Also rasterises the `.blend` file icon (artwork/spindle_blend_file.svg) that build.rs
//! puts in the Windows .exe for the files Spindle opens.
//! Self-contained (no egui) because build.rs includes this file too.

#![allow(dead_code)]

/// Logo colour (the theme accent).
pub const COLOR: [u8; 3] = [0xf2, 0x85, 0x26];

/// The symbol artwork. Its single path (absolute M/C/L/Z commands) is filled even-odd:
/// the hub and its three arms minus the hole.
pub const SVG: &str = include_str!("../../artwork/spindle_symbol.svg");

/// The `.blend` file icon artwork: paths of absolute M/C/L/Z commands, each filled
/// (non-zero) with the colour of its `class`, on a square view box.
pub const BLEND_FILE_SVG: &str = include_str!("../../artwork/spindle_blend_file.svg");

/// The value of attribute `name` in `tag` (`name="value"`).
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

/// The `d` attribute of the artwork's path.
fn path_data() -> &'static str {
    attr(SVG, "d").unwrap_or("")
}

/// Flatten the symbol's path into closed polygons.
pub fn polygons() -> Vec<Vec<[f32; 2]>> {
    polygons_of(path_data())
}

/// Flatten path data into closed polygons (cubic segments split into straight pieces).
fn polygons_of(d: &str) -> Vec<Vec<[f32; 2]>> {
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
    for ch in d.chars() {
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
    fill(&polys, [k, ox, oy], size, false)
}

/// Coverage (0..1) of `polys` scaled by `t[0]` and moved by `(t[1], t[2])` in a
/// `size`×`size` square. Scanline fill, even-odd or non-zero, 4 sub-rows per pixel with
/// exact horizontal coverage.
fn fill(polys: &[Vec<[f32; 2]>], t: [f32; 3], size: u32, nonzero: bool) -> Vec<f32> {
    let [k, ox, oy] = t;
    let n = size as f32;
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
    // Crossings of a sub-row: x and the edge's direction (+1 down, -1 up).
    let mut xs: Vec<(f32, i32)> = Vec::new();
    let mut spans: Vec<(f32, f32)> = Vec::new();
    for py in 0..size {
        let row = &mut cov[(py * size) as usize..((py + 1) * size) as usize];
        for s in 0..SUB {
            let y = py as f32 + (s as f32 + 0.5) / SUB as f32;
            xs.clear();
            for (a, b) in &edges {
                if (a[1] <= y && y < b[1]) || (b[1] <= y && y < a[1]) {
                    let dir = if a[1] < b[1] { 1 } else { -1 };
                    xs.push((a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]), dir));
                }
            }
            xs.sort_by(|p, q| p.0.total_cmp(&q.0));
            spans.clear();
            let mut winding = 0;
            for (i, &(x, dir)) in xs.iter().enumerate() {
                winding += if nonzero { dir } else { 1 };
                let inside = if nonzero { winding != 0 } else { winding % 2 == 1 };
                if let (true, Some(&(next, _))) = (inside, xs.get(i + 1)) {
                    spans.push((x, next));
                }
            }
            for &(sx0, sx1) in &spans {
                let (x0, x1) = (sx0.max(0.0), sx1.min(n));
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

/// RGBA pixels (straight alpha) of the `.blend` file icon, its view box fitted to a
/// `size`×`size` square.
pub fn rasterize_blend_file(size: u32) -> Vec<u8> {
    rasterize_svg(BLEND_FILE_SVG, size)
}

/// RGBA pixels (straight alpha) of artwork made of `<path d=".." class="..">` elements
/// filled (non-zero) with the `.class{fill:#rrggbb}` colours of its style sheet, painted
/// in order, the view box fitted to a `size`×`size` square.
fn rasterize_svg(svg: &str, size: u32) -> Vec<u8> {
    let vb: Vec<f32> = attr(svg, "viewBox")
        .unwrap_or("")
        .split([' ', ','])
        .filter_map(|v| v.parse().ok())
        .collect();
    let [vx, vy, vw, vh] = vb[..] else { panic!("artwork without a viewBox") };
    let n = size as f32;
    let k = n / vw.max(vh);
    let t = [k, (n - vw * k) / 2.0 - vx * k, (n - vh * k) / 2.0 - vy * k];
    let colour = |class: &str| -> [f32; 3] {
        let key = format!(".{class}{{fill:#");
        let hex = svg.find(&key).and_then(|i| svg.get(i + key.len()..i + key.len() + 6)).unwrap_or("000000");
        let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
        [c(0), c(2), c(4)]
    };
    // Premultiplied colour and alpha, each path painted over the ones before it.
    let mut acc = vec![[0.0f32; 4]; (size * size) as usize];
    for tag in svg.split("<path").skip(1) {
        let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
        let Some(d) = attr(tag, "d") else { continue };
        let rgb = colour(attr(tag, "class").unwrap_or(""));
        for (px, c) in acc.iter_mut().zip(fill(&polygons_of(d), t, size, true)) {
            let a = c.clamp(0.0, 1.0);
            for i in 0..3 {
                px[i] = rgb[i] * a + px[i] * (1.0 - a);
            }
            px[3] = a + px[3] * (1.0 - a);
        }
    }
    let mut out = Vec::with_capacity(acc.len() * 4);
    for [r, g, b, a] in acc {
        let un = |v: f32| if a > 0.0 { (v / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        out.extend_from_slice(&[un(r), un(g), un(b), (a * 255.0).round() as u8]);
    }
    out
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
