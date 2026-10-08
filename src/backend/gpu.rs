//! Which graphics chips this computer has, read from `/sys` (instant, and it does not wake a sleeping NVIDIA chip the
//! way `lspci` does: on a hybrid laptop that call took over two seconds and froze the page that made it).

/// PCI vendor ids.
const INTEL: u32 = 0x8086;
const AMD: u32 = 0x1002;
const NVIDIA: u32 = 0x10de;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Gpus {
    pub intel: bool,
    pub amd: bool,
    pub nvidia: bool,
}

impl Gpus {
    pub fn count(&self) -> usize {
        [self.intel, self.amd, self.nvidia].iter().filter(|b| **b).count()
    }

    pub fn from_vendor_ids(ids: &[u32]) -> Gpus {
        Gpus { intel: ids.contains(&INTEL), amd: ids.contains(&AMD), nvidia: ids.contains(&NVIDIA) }
    }
}

/// `0x8086\n` -> 0x8086.
pub fn parse_vendor(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// Vendors of the display adapters (`/sys/class/drm/cardN`, not the `cardN-HDMI-A-1` connectors).
pub fn detect() -> Gpus {
    let mut ids = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/sys/class/drm") {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let is_card = name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !is_card {
                continue;
            }
            if let Some(id) = std::fs::read_to_string(e.path().join("device/vendor")).ok().as_deref().and_then(parse_vendor) {
                ids.push(id);
            }
        }
    }
    Gpus::from_vendor_ids(&ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_ids_are_read_from_sysfs_text() {
        assert_eq!(parse_vendor("0x8086\n"), Some(0x8086));
        assert_eq!(parse_vendor("0x10de"), Some(NVIDIA));
        assert_eq!(parse_vendor("nonsense"), None);
    }

    #[test]
    fn a_hybrid_laptop_has_two_chips_and_amd_only_has_one() {
        let hybrid = Gpus::from_vendor_ids(&[NVIDIA, INTEL]);
        assert_eq!(hybrid, Gpus { intel: true, amd: false, nvidia: true });
        assert_eq!(hybrid.count(), 2);
        assert_eq!(Gpus::from_vendor_ids(&[AMD]).count(), 1);
        assert_eq!(Gpus::from_vendor_ids(&[0x1234]), Gpus::default());
    }

    /// `cargo test gpu::tests::detect_here -- --nocapture`
    #[test]
    fn detect_here() {
        println!("{:?}", detect());
    }
}
