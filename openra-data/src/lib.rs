//! File format parsers for OpenRA data files.
//!
//! - `.orarep` replay files (order stream)
//! - `.oramap` map files (terrain, actors, resources)
//! - SHP/TMP sprite files (unit/building graphics)
//! - Palette files (color lookup tables)

pub mod oramap;
pub mod orarep;
