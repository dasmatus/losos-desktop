//! GUIDs as GPT and UEFI store them.
//!
//! A GUID is written as text with its first three groups big-endian, and
//! stored on disk, in a UEFI device path and in Windows' `GUID` struct with
//! those three groups little-endian. Getting that wrong builds and runs, and
//! names a partition that does not exist, so the two forms are kept apart by
//! type: [`Guid`] holds the text order, and [`Guid::to_mixed_endian`] is the
//! only way to the stored one.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    /// Parses `8484680c-9521-48c6-9c11-b0720656f69e`, in either case and
    /// with or without braces.
    pub fn parse(text: &str) -> Option<Guid> {
        let text = text.trim().trim_start_matches('{').trim_end_matches('}');
        let groups: Vec<&str> = text.split('-').collect();
        let lengths = [8, 4, 4, 4, 12];
        if groups.len() != 5 || groups.iter().zip(lengths).any(|(g, n)| g.len() != n) {
            return None;
        }
        let hex: String = groups.concat();
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(Guid(bytes))
    }

    /// The 16 bytes as GPT entries, UEFI device paths and Windows hold them.
    pub fn to_mixed_endian(self) -> [u8; 16] {
        let b = self.0;
        [
            b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13],
            b[14], b[15],
        ]
    }

    pub fn from_mixed_endian(b: [u8; 16]) -> Guid {
        // The swap is its own inverse.
        Guid(Guid(b).to_mixed_endian())
    }

    /// The first three groups as numbers and the rest as bytes, the shape of
    /// Windows' `GUID`.
    pub fn fields(self) -> (u32, u16, u16, [u8; 8]) {
        let b = self.0;
        (
            u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_be_bytes([b[4], b[5]]),
            u16::from_be_bytes([b[6], b[7]]),
            [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        )
    }

    pub fn from_fields(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&data1.to_be_bytes());
        b[4..6].copy_from_slice(&data2.to_be_bytes());
        b[6..8].copy_from_slice(&data3.to_be_bytes());
        b[8..16].copy_from_slice(&data4);
        Guid(b)
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

/// Partition types, as `systemd-id128 show` lists them.
pub mod types {
    use super::Guid;

    fn known(text: &str) -> Guid {
        Guid::parse(text).expect("a GUID literal")
    }

    pub fn esp() -> Guid {
        known("c12a7328-f81f-11d2-ba4b-00a0c93ec93b")
    }
    /// The Boot Loader Specification's extended boot loader partition, where
    /// the UKIs go when the ESP is Windows' own and too small for two.
    pub fn xbootldr() -> Guid {
        known("bc13c2ff-59e6-4262-a352-b275fd6f7172")
    }
    /// What Windows formats C: as.
    pub fn microsoft_basic_data() -> Guid {
        known("ebd0a0a2-b9e5-4433-87c0-68b6b72699c7")
    }
    pub fn home() -> Guid {
        known("933ac7e1-2eb4-4f13-b844-0e14e2aef915")
    }
    pub fn swap() -> Guid {
        known("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_text() {
        let text = "8484680c-9521-48c6-9c11-b0720656f69e";
        assert_eq!(Guid::parse(text).unwrap().to_string(), text);
        assert_eq!(
            Guid::parse("{8484680C-9521-48C6-9C11-B0720656F69E}")
                .unwrap()
                .to_string(),
            text
        );
        assert!(Guid::parse("8484680c-9521-48c6-9c11").is_none());
        assert!(Guid::parse("8484680c-9521-48c6-9c11-b0720656f69g").is_none());
    }

    #[test]
    fn stores_the_first_three_groups_little_endian() {
        // The ESP's type as it appears in a GPT entry on disk.
        let stored = types::esp().to_mixed_endian();
        assert_eq!(
            stored,
            [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b
            ]
        );
        assert_eq!(Guid::from_mixed_endian(stored), types::esp());
    }

    #[test]
    fn fields_match_windows_guid() {
        let (d1, d2, d3, d4) = types::esp().fields();
        assert_eq!((d1, d2, d3), (0xc12a7328, 0xf81f, 0x11d2));
        assert_eq!(d4, [0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b]);
        assert_eq!(Guid::from_fields(d1, d2, d3, d4), types::esp());
    }
}
