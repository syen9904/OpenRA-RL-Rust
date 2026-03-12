//! Palette file parser (.pal).
//!
//! Parses 256-color palette files used by Red Alert.
//! Each palette is 256 entries of 3 bytes (R, G, B).
//!
//! Reference: OpenRA.Game/Graphics/HardwarePalette.cs

/// A 256-color palette.
#[derive(Debug, Clone)]
pub struct Palette {
    /// 256 RGB entries. Each entry is [R, G, B].
    pub colors: [[u8; 3]; 256],
}

impl Palette {
    /// Parse a .pal file (768 bytes: 256 * 3 RGB).
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < 768 {
            return Err(format!(
                "Palette file too small: {} bytes (expected 768)",
                data.len()
            ));
        }

        let mut colors = [[0u8; 3]; 256];
        for i in 0..256 {
            let base = i * 3;
            // RA palettes use 6-bit color (0-63), scale to 8-bit
            colors[i] = [
                scale_6bit(data[base]),
                scale_6bit(data[base + 1]),
                scale_6bit(data[base + 2]),
            ];
        }

        Ok(Palette { colors })
    }

    /// Parse a .pal file without 6-bit scaling (already 8-bit).
    pub fn from_bytes_8bit(data: &[u8]) -> Result<Self, String> {
        if data.len() < 768 {
            return Err(format!(
                "Palette file too small: {} bytes (expected 768)",
                data.len()
            ));
        }

        let mut colors = [[0u8; 3]; 256];
        for i in 0..256 {
            let base = i * 3;
            colors[i] = [data[base], data[base + 1], data[base + 2]];
        }

        Ok(Palette { colors })
    }

    /// Get an RGBA color (with alpha = 255, except index 0 which is transparent).
    pub fn rgba(&self, index: u8) -> [u8; 4] {
        if index == 0 {
            [0, 0, 0, 0] // Transparent
        } else {
            let [r, g, b] = self.colors[index as usize];
            [r, g, b, 255]
        }
    }

    /// Apply a player color remap to a range of palette indices.
    /// Returns a new palette with the remap applied.
    pub fn with_remap(&self, remap_start: u8, remap_end: u8, player_colors: &[[u8; 3]]) -> Self {
        let mut result = self.clone();
        let range_len = (remap_end - remap_start + 1) as usize;
        for (i, color) in player_colors.iter().take(range_len).enumerate() {
            result.colors[(remap_start as usize) + i] = *color;
        }
        result
    }
}

/// Scale a 6-bit palette value (0-63) to 8-bit (0-255).
fn scale_6bit(val: u8) -> u8 {
    let v = val & 0x3F; // Mask to 6 bits
    (v << 2) | (v >> 4) // Scale: multiply by ~4.047
}

/// Standard RA player color remap range (palette indices 80-95).
pub const REMAP_START: u8 = 80;
pub const REMAP_END: u8 = 95;

/// Standard player colors for RA (approximate).
pub const PLAYER_COLORS: [[u8; 3]; 8] = [
    [255, 255, 0],   // Yellow
    [0, 255, 255],   // Cyan
    [0, 255, 0],     // Green
    [255, 128, 0],   // Orange
    [128, 128, 128], // Grey
    [255, 0, 0],     // Red
    [0, 0, 255],     // Blue
    [255, 0, 255],   // Purple
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_6bit_values() {
        assert_eq!(scale_6bit(0), 0);
        assert_eq!(scale_6bit(63), 255);
        assert_eq!(scale_6bit(32), 130); // 32*4 + 32/16 = 128+2 = 130
    }

    #[test]
    fn parse_palette() {
        let mut data = vec![0u8; 768];
        // Set index 1 to (63, 0, 32) in 6-bit
        data[3] = 63;
        data[4] = 0;
        data[5] = 32;

        let pal = Palette::from_bytes(&data).unwrap();
        assert_eq!(pal.colors[0], [0, 0, 0]);
        assert_eq!(pal.colors[1], [255, 0, 130]);
    }

    #[test]
    fn transparent_index_zero() {
        let data = vec![0u8; 768];
        let pal = Palette::from_bytes(&data).unwrap();
        assert_eq!(pal.rgba(0), [0, 0, 0, 0]);
    }

    #[test]
    fn palette_too_small() {
        let data = vec![0u8; 100];
        assert!(Palette::from_bytes(&data).is_err());
    }
}
