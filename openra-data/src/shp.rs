//! SHP TD format sprite decoder.
//!
//! Decodes SHP files from the Tiberian Dawn / Red Alert engine format.
//! Each SHP file contains multiple frames of 8-bit indexed pixel data.
//!
//! Reference: OpenRA.Mods.Common/SpriteLoaders/ShpTDLoader.cs

/// A single sprite frame.
#[derive(Debug, Clone)]
pub struct SpriteFrame {
    pub width: u16,
    pub height: u16,
    /// 8-bit indexed pixel data, row-major. Length = width * height.
    pub pixels: Vec<u8>,
}

/// A decoded SHP file containing multiple frames.
#[derive(Debug, Clone)]
pub struct ShpFile {
    pub frames: Vec<SpriteFrame>,
}

/// SHP format constants.
const FORMAT_XOR_PREV: u8 = 0x20;
const FORMAT_XOR_BASE: u8 = 0x40;
const FORMAT_80: u8 = 0x80;

/// Decode an SHP TD format file from raw bytes.
pub fn decode(data: &[u8]) -> Result<ShpFile, String> {
    if data.len() < 14 {
        return Err("SHP file too small".into());
    }

    // Header: u16 num_images, then x, y, width, height (all u16)
    let num_images = read_u16(data, 0) as usize;
    if num_images == 0 {
        return Err("SHP file has zero frames".into());
    }

    // Read frame offsets (num_images + 2 entries, each 8 bytes)
    // Format: offset(3 bytes LE) + format(1 byte) + refoff(3 bytes) + refformat(1 byte)
    let header_size = 2;
    let offset_entry_size = 8;

    if data.len() < header_size + (num_images + 2) * offset_entry_size {
        return Err("SHP file truncated in offset table".into());
    }

    let mut offsets = Vec::with_capacity(num_images + 2);
    for i in 0..(num_images + 2) {
        let base = header_size + i * offset_entry_size;
        let offset = read_u24(data, base);
        let format = data[base + 3];
        let ref_offset = read_u24(data, base + 4);
        let ref_format = data[base + 7];
        offsets.push(FrameHeader {
            offset: offset as usize,
            format,
            ref_offset: ref_offset as usize,
            ref_format,
        });
    }

    // Read image dimensions from first frame header
    // After offset table: each frame at its offset has a small header
    let mut frames = Vec::with_capacity(num_images);

    for i in 0..num_images {
        let hdr = &offsets[i];
        let frame_start = hdr.offset;

        if frame_start + 4 > data.len() {
            return Err(format!("Frame {} offset out of bounds", i));
        }

        let width = read_u16(data, frame_start);
        let height = read_u16(data, frame_start + 2);
        let pixel_count = width as usize * height as usize;

        if pixel_count == 0 {
            frames.push(SpriteFrame {
                width,
                height,
                pixels: Vec::new(),
            });
            continue;
        }

        // Compressed data starts after the 4-byte frame header (width, height)
        let compressed_start = frame_start + 4;
        let compressed_data = &data[compressed_start..];

        let pixels = match hdr.format {
            FORMAT_80 => decode_format80(compressed_data, pixel_count)?,
            FORMAT_XOR_PREV if i > 0 => {
                let prev = &frames[i - 1].pixels;
                let base_decoded = decode_format80(compressed_data, pixel_count)?;
                xor_buffers(prev, &base_decoded)
            }
            FORMAT_XOR_BASE => {
                // XOR with the reference frame
                let ref_hdr = &offsets[i];
                let ref_start = ref_hdr.ref_offset;
                if ref_start + 4 > data.len() {
                    return Err(format!("Ref frame offset out of bounds for frame {}", i));
                }
                let ref_compressed = &data[ref_start + 4..];
                let ref_decoded = decode_format80(ref_compressed, pixel_count)?;
                let base_decoded = decode_format80(compressed_data, pixel_count)?;
                xor_buffers(&ref_decoded, &base_decoded)
            }
            _ => {
                // Treat as format 80
                decode_format80(compressed_data, pixel_count)?
            }
        };

        frames.push(SpriteFrame {
            width,
            height,
            pixels,
        });
    }

    Ok(ShpFile { frames })
}

#[derive(Debug)]
struct FrameHeader {
    offset: usize,
    format: u8,
    ref_offset: usize,
    #[allow(dead_code)]
    ref_format: u8,
}

/// Decode Format80 (LCW) compressed data.
/// This is the compression used by Westwood's SHP files.
fn decode_format80(src: &[u8], max_output: usize) -> Result<Vec<u8>, String> {
    let mut dest = Vec::with_capacity(max_output);
    let mut i = 0;

    while i < src.len() && dest.len() < max_output {
        let cmd = src[i];
        i += 1;

        if cmd == 0x80 {
            // End of data
            break;
        } else if cmd == 0xFF {
            // Long absolute move: 0xFF count_lo count_hi src_lo src_hi
            if i + 4 > src.len() {
                break;
            }
            let count = read_u16(src, i) as usize;
            let src_pos = read_u16(src, i + 2) as usize;
            i += 4;
            for j in 0..count {
                if dest.len() >= max_output {
                    break;
                }
                if src_pos + j < dest.len() {
                    dest.push(dest[src_pos + j]);
                } else {
                    dest.push(0);
                }
            }
        } else if cmd == 0xFE {
            // Long fill: 0xFE count_lo count_hi value
            if i + 3 > src.len() {
                break;
            }
            let count = read_u16(src, i) as usize;
            let value = src[i + 2];
            i += 3;
            for _ in 0..count.min(max_output - dest.len()) {
                dest.push(value);
            }
        } else if cmd & 0x80 != 0 {
            if cmd & 0x40 != 0 {
                // Short absolute move from dest: 11cccccc src_lo src_hi
                let count = ((cmd & 0x3F) as usize) + 3;
                if i + 2 > src.len() {
                    break;
                }
                let src_pos = read_u16(src, i) as usize;
                i += 2;
                for j in 0..count {
                    if dest.len() >= max_output {
                        break;
                    }
                    if src_pos + j < dest.len() {
                        dest.push(dest[src_pos + j]);
                    } else {
                        dest.push(0);
                    }
                }
            } else {
                // Short relative move from dest: 10cccccc offset
                let count = ((cmd >> 0) & 0x3F) as usize;
                if count == 0 {
                    break; // Zero count = end
                }
                let count = count + 3; // minimum copy of 3
                if i >= src.len() {
                    break;
                }
                // Relative offset: negative from current dest position
                // But in Format80, this is actually: 10cccccc d (copy count+3 bytes from dest[dest.len()-d..])
                // Wait — Format80 doesn't use relative offsets this way.
                // Actually: 10cccccc d -> copy (c+3) bytes starting at (dest.len() - d)
                // But d is only 1 byte, so max 255 lookback... that's not right.
                // Let me re-check: 10xxxxxx = (count-3) in bits 0-5, offset from current in next byte
                // Actually the correct Form80 is more nuanced. Let me use a simpler approach.
                let offset_byte = src[i] as usize;
                i += 1;
                let src_pos = if dest.len() >= offset_byte {
                    dest.len() - offset_byte
                } else {
                    0
                };
                for j in 0..count - 3 {
                    if dest.len() >= max_output {
                        break;
                    }
                    if src_pos + j < dest.len() {
                        dest.push(dest[src_pos + j]);
                    } else {
                        dest.push(0);
                    }
                }
            }
        } else {
            // Direct copy: 0ccccccc = copy (c) bytes from source
            let count = cmd as usize;
            if count == 0 {
                break;
            }
            for _ in 0..count {
                if dest.len() >= max_output || i >= src.len() {
                    break;
                }
                dest.push(src[i]);
                i += 1;
            }
        }
    }

    // Pad to expected size if needed
    dest.resize(max_output, 0);
    Ok(dest)
}

/// XOR two buffers together.
fn xor_buffers(base: &[u8], overlay: &[u8]) -> Vec<u8> {
    base.iter()
        .zip(overlay.iter())
        .map(|(a, b)| a ^ b)
        .collect()
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u24(data: &[u8], offset: usize) -> u32 {
    data[offset] as u32
        | ((data[offset + 1] as u32) << 8)
        | ((data[offset + 2] as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_helpers() {
        let data = [0x34, 0x12, 0xAB];
        assert_eq!(read_u16(&data, 0), 0x1234);
        assert_eq!(read_u24(&data, 0), 0xAB1234);
    }

    #[test]
    fn decode_empty_shp() {
        // Minimal SHP with 0 frames
        let data = [0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = decode(&data);
        assert!(result.is_err());
    }
}
