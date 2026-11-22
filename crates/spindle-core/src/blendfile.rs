//! Opening a `.blend` file: read which Blender saved it and pick the build to open it with.

use crate::Spindle;
use crate::builds::{self, Build};
use crate::compat::{BlendHeader, BlendRead, parse_blend_header};
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// The header of a .blend file, decompressing zstd (Blender 3.0+) or gzip (older) files.
pub fn read_header(path: &Path) -> Result<BlendRead> {
    let open = || File::open(path).with_context(|| format!("opening {}", path.display()));
    let mut magic = [0u8; 4];
    let n = read_up_to(open()?, &mut magic)?;
    header_of(std::io::BufReader::new(open()?), &magic[..n])
}

/// Header of a .blend stream whose first bytes are `magic`.
pub fn header_of(r: impl Read, magic: &[u8]) -> Result<BlendRead> {
    let mut out = [0u8; 32];
    let got = if magic.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        let d = ruzstd::decoding::StreamingDecoder::new(r).map_err(|e| anyhow::anyhow!("reading zstd stream: {e}"))?;
        read_up_to(d, &mut out)?
    } else if magic.starts_with(&[0x1F, 0x8B]) {
        read_up_to(flate2::read::GzDecoder::new(r), &mut out)?
    } else {
        read_up_to(r, &mut out)?
    };
    Ok(parse_blend_header(&out[..got]))
}

fn read_up_to(mut r: impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// The build chosen for a file, and why.
#[derive(Debug, Clone)]
pub struct Choice {
    pub build: Build,
    /// Series of the Blender that saved the file, when readable.
    pub saved_by: Option<(u32, u32)>,
    /// Something the user should confirm before opening (e.g. the file is newer than every build).
    pub warning: Option<String>,
}

/// Pick the build to open a .blend file with, preferring builds that can use the profile
/// (`usable`; the others are used only when no usable build can open the file):
/// 1. a build of the series that saved it (a favourite of that series first, else its newest);
/// 2. else the default build (first favourite, else newest) if it's newer than the file;
/// 3. else the oldest build newer than the file;
/// 4. else (the file is newer than every build) the newest build, with a warning.
///
/// Files whose header can't be read open in the default build.
pub fn choose_build(sp: &Spindle, path: &Path, usable: impl Fn(&Build) -> bool) -> Result<Choice> {
    let list = builds::list(sp)?;
    if list.is_empty() {
        bail!("no builds yet (download one with `spindle builds download <version>`)");
    }
    let header = match read_header(path)? {
        BlendRead::Header(h) => Some(h),
        BlendRead::Compressed => None,
        BlendRead::NotBlend => bail!("{} is not a .blend file", path.display()),
    };
    Ok(choose_from(&list, header, usable))
}

fn by_version_desc(v: &mut [&Build]) {
    v.sort_by(|a, b| crate::version::compare(b.version.as_deref().unwrap_or(""), a.version.as_deref().unwrap_or("")));
}

fn choose_from(list: &[Build], header: Option<BlendHeader>, usable: impl Fn(&Build) -> bool) -> Choice {
    // `list` is ordered favourites first, then newest first.
    let best = |cands: &[&Build]| cands.iter().find(|b| usable(b)).or(cands.first()).map(|b| (*b).clone());
    let Some(h) = header else {
        let all: Vec<&Build> = list.iter().collect();
        return Choice { build: best(&all).unwrap(), saved_by: None, warning: None };
    };
    // Builds that can open the file, best first.
    let mut cands: Vec<&Build> = list.iter().filter(|b| b.series() == Some(h.series)).collect();
    if list[0].series().is_some_and(|s| s > h.series) {
        cands.push(&list[0]);
    }
    let mut newer: Vec<&Build> = list.iter().filter(|b| b.series().is_some_and(|s| s > h.series)).collect();
    by_version_desc(&mut newer);
    cands.extend(newer.into_iter().rev()); // oldest newer build first: closest to the file
    if let Some(build) = best(&cands) {
        return Choice { build, saved_by: Some(h.series), warning: None };
    }
    let mut all: Vec<&Build> = list.iter().collect();
    by_version_desc(&mut all);
    let newest = all[0];
    let (fm, fn_) = h.series;
    let have = newest.series().map(|(a, b)| format!("{a}.{b}")).unwrap_or("?".into());
    let warning = if h.new_format && newest.series().is_some_and(|s| s < crate::compat::READS_NEW_BLEND_FORMAT) {
        format!("This file was saved by Blender {fm}.{fn_} in the 5.0+ file format, which Blender {have} can't read. Download Blender {fm}.{fn_} or newer to open it.")
    } else {
        format!("This file was saved by Blender {fm}.{fn_}, newer than every build in the library (newest: {have}). Opening it in an older Blender can lose data that version doesn't know about if you save.")
    };
    Choice { build: newest.clone(), saved_by: Some(h.series), warning: Some(warning) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builds::Channel;
    use std::io::Write;

    fn build(name: &str, version: &str, favourite: bool) -> Build {
        Build {
            name: name.into(),
            executable: format!("/b/{name}/blender").into(),
            version: Some(version.into()),
            external: false,
            channel: Channel::Stable,
            meta: None,
            favourite,
        }
    }

    fn h(a: u32, b: u32) -> Option<BlendHeader> {
        Some(BlendHeader { series: (a, b), new_format: (a, b) >= (5, 0) })
    }

    #[test]
    fn picks_matching_series_then_default_then_newest() {
        // Ordered like builds::list: favourites first, then newest.
        let list = vec![build("fav-4.5", "4.5.16", true), build("b-5.3", "5.3.0", false), build("b-4.1", "4.1.1", false)];
        let any = |_: &Build| true;
        assert_eq!(choose_from(&list, h(4, 1), any).build.name, "b-4.1");
        assert_eq!(choose_from(&list, h(4, 2), any).build.name, "fav-4.5");
        assert_eq!(choose_from(&list, h(5, 2), any).build.name, "b-5.3");
        assert!(choose_from(&list, h(5, 2), any).warning.is_none());
        let newer = choose_from(&list, h(5, 4), any);
        assert_eq!(newer.build.name, "b-5.3");
        assert!(newer.warning.unwrap().contains("newer than every build"));
        assert_eq!(choose_from(&list, None, any).build.name, "fav-4.5");
        let old = vec![build("b-4.1", "4.1.1", false)];
        assert!(choose_from(&old, h(5, 0), any).warning.unwrap().contains("can't read"));
        // A 4.1 file when the 4.1 build can't use the profile: the default (4.5) instead.
        assert_eq!(choose_from(&list, h(4, 1), |b: &Build| b.name != "b-4.1").build.name, "fav-4.5");
        // Nothing usable: still the closest match.
        assert_eq!(choose_from(&list, h(4, 1), |_: &Build| false).build.name, "b-4.1");
    }

    #[test]
    fn reads_compressed_headers() {
        let plain = b"BLENDER17-01v0503REND\0\0\0\0\0\0\0\0\0\0\0";
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(plain).unwrap();
        let gz = gz.finish().unwrap();
        let want = BlendRead::Header(BlendHeader { series: (5, 3), new_format: true });
        assert_eq!(header_of(&gz[..], &gz[..4]).unwrap(), want);
        let zst = ruzstd::encoding::compress_to_vec(&plain[..], ruzstd::encoding::CompressionLevel::Fastest);
        assert_eq!(header_of(&zst[..], &zst[..4]).unwrap(), want);
        let legacy = b"BLENDER-v405REND";
        assert_eq!(header_of(&legacy[..], &legacy[..4]).unwrap(), BlendRead::Header(BlendHeader { series: (4, 5), new_format: false }));
    }
}
