//! The icon of a Windows program: read from the `.exe` itself (its resources), so a game added from a file shows its own
//! picture instead of a generic one. No outside tool is needed: the file's resource tree is read here, and an icon that
//! is not already a PNG is converted with the picture library the app already has.

use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn u16_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?) as u32)
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

struct Pe<'a> {
    data: &'a [u8],
    /// (virtual address, virtual size, file offset, raw size)
    sections: Vec<(u32, u32, u32, u32)>,
    rsrc_rva: u32,
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Option<Pe<'a>> {
        if data.get(0..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(data, 0x3c)? as usize;
        if data.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let nsec = u16_at(data, pe + 6)? as usize;
        let opt_size = u16_at(data, pe + 20)? as usize;
        let opt = pe + 24;
        let dir = match u16_at(data, opt)? {
            0x10b => opt + 96,
            0x20b => opt + 112,
            _ => return None,
        };
        let rsrc_rva = u32_at(data, dir + 2 * 8)?;
        let mut sections = Vec::new();
        for i in 0..nsec {
            let s = opt + opt_size + i * 40;
            sections.push((u32_at(data, s + 12)?, u32_at(data, s + 8)?, u32_at(data, s + 20)?, u32_at(data, s + 16)?));
        }
        Some(Pe { data, sections, rsrc_rva })
    }

    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find(|(va, vs, _, raw)| rva >= *va && rva < va + (*vs).max(*raw)).map(|(va, _, off, _)| (rva - va + off) as usize)
    }

    /// Every resource of `kind`: (id or 0, bytes). Walks type > name > language.
    fn resources(&self, kind: u32) -> Vec<(u32, &'a [u8])> {
        let mut out = Vec::new();
        let Some(root) = self.offset(self.rsrc_rva) else { return out };
        let entries = |dir: usize| -> Vec<(u32, u32)> {
            let n = u16_at(self.data, dir + 12).unwrap_or(0) + u16_at(self.data, dir + 14).unwrap_or(0);
            (0..n as usize).filter_map(|i| Some((u32_at(self.data, dir + 16 + i * 8)?, u32_at(self.data, dir + 20 + i * 8)?))).collect()
        };
        for (tid, toff) in entries(root) {
            if tid != kind || toff & 0x8000_0000 == 0 {
                continue;
            }
            for (nid, noff) in entries(root + (toff & 0x7fff_ffff) as usize) {
                if noff & 0x8000_0000 == 0 {
                    continue;
                }
                // the first language is enough
                let Some((_, loff)) = entries(root + (noff & 0x7fff_ffff) as usize).into_iter().next() else { continue };
                if loff & 0x8000_0000 != 0 {
                    continue;
                }
                let de = root + loff as usize;
                let (Some(rva), Some(size)) = (u32_at(self.data, de), u32_at(self.data, de + 4)) else { continue };
                if let Some(o) = self.offset(rva) {
                    if let Some(bytes) = self.data.get(o..o + size as usize) {
                        out.push((nid & 0xffff, bytes));
                    }
                }
            }
        }
        out
    }
}

/// The program's biggest icon as an `.ico` file's bytes (a single image), or as PNG bytes when it already is one.
/// The second value says which it is.
pub fn biggest_icon(exe: &[u8]) -> Option<(Vec<u8>, bool)> {
    let pe = Pe::parse(exe)?;
    let groups = pe.resources(RT_GROUP_ICON);
    let (_, group) = groups.first()?;
    let count = u16_at(group, 4)? as usize;
    // entries: width, height, colours, reserved, planes(2), bits(2), size(4), id(2)
    let (mut best_id, mut best_w, mut best_bits) = (0u32, 0u32, 0u32);
    for i in 0..count {
        let e = 6 + i * 14;
        let w = match *group.get(e)? {
            0 => 256,
            w => w as u32,
        };
        let bits = u16_at(group, e + 6)?;
        if (w, bits) > (best_w, best_bits) {
            (best_id, best_w, best_bits) = (u16_at(group, e + 12)?, w, bits);
        }
    }
    let icons = pe.resources(RT_ICON);
    let (_, img) = icons.iter().find(|(id, _)| *id == best_id)?;
    if img.starts_with(&PNG) {
        return Some((img.to_vec(), true));
    }
    let w = if best_w >= 256 { 0 } else { best_w as u8 };
    let mut ico = vec![0, 0, 1, 0, 1, 0, w, w, 0, 0, 1, 0, best_bits as u8, 0];
    ico.extend((img.len() as u32).to_le_bytes());
    ico.extend(22u32.to_le_bytes());
    ico.extend_from_slice(img);
    Some((ico, false))
}

/// Writes the program's icon as a PNG at `out`. Err = why not (the caller keeps the generic icon).
pub fn extract_png(exe: &Path, out: &Path) -> Result<(), String> {
    let bytes = std::fs::read(exe).map_err(|e| e.to_string())?;
    let (img, is_png) = biggest_icon(&bytes).ok_or("This program has no icon of its own.")?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if is_png {
        return std::fs::write(out, img).map_err(|e| e.to_string());
    }
    use gtk4::gdk_pixbuf::prelude::*;
    let loader = gtk4::gdk_pixbuf::PixbufLoader::with_type("ico").map_err(|e| e.to_string())?;
    loader.write(&img).map_err(|e| e.to_string())?;
    loader.close().map_err(|e| e.to_string())?;
    let pix = loader.pixbuf().ok_or("The icon could not be read.")?;
    pix.savev(out, "png", &[]).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny PE file whose only resources are one PNG icon and its group.
    fn fake_pe(icon: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 0x400];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        f[0x80..0x84].copy_from_slice(b"PE\0\0");
        f[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // one section
        f[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes()); // optional header size
        let opt = 0x98;
        f[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
        f[opt + 96 + 16..opt + 96 + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // resource directory RVA
        let sec = opt + 0xe0;
        f[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual size
        f[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual address
        f[sec + 16..sec + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // raw size
        f[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes()); // raw offset
        // resource section at file offset 0x200 (RVA 0x1000)
        let mut r = vec![0u8; 0x600];
        let put32 = |r: &mut Vec<u8>, o: usize, v: u32| r[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let put16 = |r: &mut Vec<u8>, o: usize, v: u16| r[o..o + 2].copy_from_slice(&v.to_le_bytes());
        // root: two ID entries: type 3 (icon) -> 0x20, type 14 (group) -> 0x80
        put16(&mut r, 14, 2);
        put32(&mut r, 16, 3);
        put32(&mut r, 20, 0x8000_0000 | 0x20);
        put32(&mut r, 24, 14);
        put32(&mut r, 28, 0x8000_0000 | 0x80);
        // icon type dir at 0x20: one entry id 1 -> 0x40 (language dir)
        put16(&mut r, 0x20 + 14, 1);
        put32(&mut r, 0x20 + 16, 1);
        put32(&mut r, 0x20 + 20, 0x8000_0000 | 0x40);
        // language dir at 0x40: one entry -> data entry at 0x60
        put16(&mut r, 0x40 + 14, 1);
        put32(&mut r, 0x40 + 16, 0x409);
        put32(&mut r, 0x40 + 20, 0x60);
        // group type dir at 0x80: id 1 -> 0xa0 ; language dir at 0xa0 -> data entry at 0xc0
        put16(&mut r, 0x80 + 14, 1);
        put32(&mut r, 0x80 + 16, 1);
        put32(&mut r, 0x80 + 20, 0x8000_0000 | 0xa0);
        put16(&mut r, 0xa0 + 14, 1);
        put32(&mut r, 0xa0 + 16, 0x409);
        put32(&mut r, 0xa0 + 20, 0xc0);
        // data entries: icon at RVA 0x1200, group at RVA 0x1100
        put32(&mut r, 0x60, 0x1200);
        put32(&mut r, 0x64, icon.len() as u32);
        put32(&mut r, 0xc0, 0x1100);
        put32(&mut r, 0xc4, 20);
        // group data at 0x100: header (count 1) + one 14-byte entry (width 0 = 256, bits 32, id 1)
        r[0x100..0x106].copy_from_slice(&[0, 0, 1, 0, 1, 0]);
        r[0x106..0x114].copy_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 1, 0]);
        r[0x200..0x200 + icon.len()].copy_from_slice(icon);
        f.truncate(0x200);
        f.extend(r);
        f
    }

    #[test]
    fn a_png_icon_is_found_in_the_resources() {
        let mut png = PNG.to_vec();
        png.extend_from_slice(b"fake image body");
        let (img, is_png) = biggest_icon(&fake_pe(&png)).unwrap();
        assert!(is_png);
        assert_eq!(img, png);
    }

    #[test]
    fn a_bitmap_icon_is_wrapped_as_an_ico_file() {
        let (img, is_png) = biggest_icon(&fake_pe(b"BMPDATA!")).unwrap();
        assert!(!is_png);
        assert_eq!(&img[0..4], &[0, 0, 1, 0]);
        assert_eq!(&img[22..], b"BMPDATA!");
        assert_eq!(u32::from_le_bytes(img[14..18].try_into().unwrap()), 8);
    }

    #[test]
    fn garbage_is_not_a_program() {
        assert!(biggest_icon(b"hello").is_none());
        assert!(biggest_icon(&[b'M', b'Z', 0, 0]).is_none());
        assert!(biggest_icon(&[]).is_none());
    }

    /// `ZS_TEST_EXE=/path/to/real.exe cargo test real_program -- --ignored`: the icon of a real program.
    #[test]
    #[ignore]
    fn a_real_program_gives_a_picture() {
        let exe = std::env::var("ZS_TEST_EXE").expect("set ZS_TEST_EXE");
        let out = std::path::PathBuf::from(std::env::var("ZS_TEST_OUT").unwrap_or("/tmp/zs-icon.png".into()));
        extract_png(Path::new(&exe), &out).unwrap();
        assert!(std::fs::metadata(&out).unwrap().len() > 100);
    }
}
