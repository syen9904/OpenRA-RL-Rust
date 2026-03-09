//! Fixed-point coordinate types matching OpenRA's WPos, WVec, WAngle, WDist, CPos.
//! All arithmetic must be bit-for-bit identical to the C# implementation.
//!
//! Reference: OpenRA.Game/WPos.cs, WVec.cs, WAngle.cs, WDist.cs, CPos.cs

use std::ops;

/// 3D world position. 1024 units = 1 cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl WPos {
    pub const ZERO: WPos = WPos { x: 0, y: 0, z: 0 };

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        WPos { x, y, z }
    }

    /// C# GetHashCode: X ^ Y ^ Z
    pub fn sync_hash(self) -> i32 {
        self.x ^ self.y ^ self.z
    }
}

impl ops::Add<WVec> for WPos {
    type Output = WPos;
    fn add(self, rhs: WVec) -> WPos {
        WPos::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl ops::Sub<WVec> for WPos {
    type Output = WPos;
    fn sub(self, rhs: WVec) -> WPos {
        WPos::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl ops::Sub for WPos {
    type Output = WVec;
    fn sub(self, rhs: WPos) -> WVec {
        WVec::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// 3D world vector (delta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WVec {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl WVec {
    pub const ZERO: WVec = WVec { x: 0, y: 0, z: 0 };

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        WVec { x, y, z }
    }

    pub fn length_squared(self) -> i64 {
        self.x as i64 * self.x as i64
            + self.y as i64 * self.y as i64
            + self.z as i64 * self.z as i64
    }

    pub fn horizontal_length_squared(self) -> i64 {
        self.x as i64 * self.x as i64 + self.y as i64 * self.y as i64
    }

    pub fn dot(a: WVec, b: WVec) -> i64 {
        a.x as i64 * b.x as i64 + a.y as i64 * b.y as i64 + a.z as i64 * b.z as i64
    }

    /// C# GetHashCode: X ^ Y ^ Z
    pub fn sync_hash(self) -> i32 {
        self.x ^ self.y ^ self.z
    }
}

impl ops::Add for WVec {
    type Output = WVec;
    fn add(self, rhs: WVec) -> WVec {
        WVec::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl ops::Sub for WVec {
    type Output = WVec;
    fn sub(self, rhs: WVec) -> WVec {
        WVec::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl ops::Neg for WVec {
    type Output = WVec;
    fn neg(self) -> WVec {
        WVec::new(-self.x, -self.y, -self.z)
    }
}

impl ops::Mul<i32> for WVec {
    type Output = WVec;
    fn mul(self, rhs: i32) -> WVec {
        WVec::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl ops::Div<i32> for WVec {
    type Output = WVec;
    fn div(self, rhs: i32) -> WVec {
        WVec::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

/// 1D angle. 1024 units = 360°. Normalized to [0, 1024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WAngle {
    pub angle: i32,
}

impl WAngle {
    pub const ZERO: WAngle = WAngle { angle: 0 };

    pub fn new(a: i32) -> Self {
        let mut angle = a % 1024;
        if angle < 0 {
            angle += 1024;
        }
        WAngle { angle }
    }

    /// C#: Angle / 4 (facing is 0-255, angle is 0-1023)
    pub fn facing(self) -> i32 {
        self.angle / 4
    }

    /// C# GetHashCode: Angle
    pub fn sync_hash(self) -> i32 {
        self.angle
    }
}

impl Default for WAngle {
    fn default() -> Self {
        Self::ZERO
    }
}

impl ops::Add for WAngle {
    type Output = WAngle;
    fn add(self, rhs: WAngle) -> WAngle {
        WAngle::new(self.angle + rhs.angle)
    }
}

impl ops::Sub for WAngle {
    type Output = WAngle;
    fn sub(self, rhs: WAngle) -> WAngle {
        WAngle::new(self.angle - rhs.angle)
    }
}

impl ops::Neg for WAngle {
    type Output = WAngle;
    fn neg(self) -> WAngle {
        WAngle::new(-self.angle)
    }
}

/// 1D world distance. 1024 units = 1 cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WDist {
    pub length: i32,
}

impl WDist {
    pub const ZERO: WDist = WDist { length: 0 };
    pub const MAX_VALUE: WDist = WDist { length: i32::MAX };

    pub const fn new(length: i32) -> Self {
        WDist { length }
    }

    pub const fn from_cells(cells: i32) -> Self {
        WDist { length: 1024 * cells }
    }

    pub fn length_squared(self) -> i64 {
        self.length as i64 * self.length as i64
    }

    /// C# GetHashCode: Length
    pub fn sync_hash(self) -> i32 {
        self.length
    }
}

impl ops::Add for WDist {
    type Output = WDist;
    fn add(self, rhs: WDist) -> WDist {
        WDist::new(self.length + rhs.length)
    }
}

impl ops::Sub for WDist {
    type Output = WDist;
    fn sub(self, rhs: WDist) -> WDist {
        WDist::new(self.length - rhs.length)
    }
}

impl ops::Mul<i32> for WDist {
    type Output = WDist;
    fn mul(self, rhs: i32) -> WDist {
        WDist::new(self.length * rhs)
    }
}

/// Cell position with packed bit representation.
///
/// Layout: XXXX_XXXX_XXXX_YYYY_YYYY_YYYY_LLLL_LLLL
/// - X: 12 bits signed (bits 31:20)
/// - Y: 12 bits signed (bits 19:8)
/// - Layer: 8 bits unsigned (bits 7:0)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CPos {
    pub bits: i32,
}

impl CPos {
    pub const ZERO: CPos = CPos { bits: 0 };

    /// Create from packed bits (as stored in replay orders)
    pub const fn from_bits(bits: i32) -> Self {
        CPos { bits }
    }

    /// Create from X, Y coordinates (layer = 0)
    pub fn new(x: i32, y: i32) -> Self {
        Self::with_layer(x, y, 0)
    }

    /// Create from X, Y, layer
    pub fn with_layer(x: i32, y: i32, layer: u8) -> Self {
        CPos {
            bits: ((x & 0xFFF) << 20) | ((y & 0xFFF) << 8) | layer as i32,
        }
    }

    /// X coordinate (12-bit signed, sign-extended via arithmetic shift)
    /// C#: Bits >> 20
    pub fn x(self) -> i32 {
        self.bits >> 20
    }

    /// Y coordinate (12-bit signed, sign-extended)
    /// C#: ((short)(Bits >> 4)) >> 4
    pub fn y(self) -> i32 {
        ((self.bits >> 4) as i16 >> 4) as i32
    }

    /// Layer (8-bit unsigned)
    /// C#: (byte)Bits
    pub fn layer(self) -> u8 {
        self.bits as u8
    }

    /// C# GetHashCode: Bits
    pub fn sync_hash(self) -> i32 {
        self.bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpos_arithmetic() {
        let a = WPos::new(100, 200, 0);
        let b = WPos::new(30, 40, 0);
        let diff = a - b;
        assert_eq!(diff, WVec::new(70, 160, 0));
        assert_eq!(b + diff, a);
    }

    #[test]
    fn wpos_hash() {
        let p = WPos::new(1024, 2048, 0);
        assert_eq!(p.sync_hash(), 1024 ^ 2048 ^ 0);
    }

    #[test]
    fn wvec_length_squared() {
        let v = WVec::new(3, 4, 0);
        assert_eq!(v.length_squared(), 25);
        assert_eq!(v.horizontal_length_squared(), 25);
    }

    #[test]
    fn wvec_dot() {
        let a = WVec::new(1, 2, 3);
        let b = WVec::new(4, 5, 6);
        assert_eq!(WVec::dot(a, b), 32);
    }

    #[test]
    fn wangle_normalization() {
        assert_eq!(WAngle::new(0).angle, 0);
        assert_eq!(WAngle::new(512).angle, 512);
        assert_eq!(WAngle::new(1024).angle, 0);
        assert_eq!(WAngle::new(1025).angle, 1);
        assert_eq!(WAngle::new(-1).angle, 1023);
        assert_eq!(WAngle::new(-1024).angle, 0);
    }

    #[test]
    fn wangle_facing() {
        assert_eq!(WAngle::new(0).facing(), 0);
        assert_eq!(WAngle::new(256).facing(), 64); // 90° = facing 64
        assert_eq!(WAngle::new(512).facing(), 128); // 180°
    }

    #[test]
    fn wangle_arithmetic() {
        let a = WAngle::new(900);
        let b = WAngle::new(200);
        assert_eq!((a + b).angle, 76); // (900 + 200) % 1024 = 76
        assert_eq!((a - b).angle, 700);
    }

    #[test]
    fn wdist_from_cells() {
        assert_eq!(WDist::from_cells(1).length, 1024);
        assert_eq!(WDist::from_cells(5).length, 5120);
    }

    #[test]
    fn cpos_pack_unpack() {
        let c = CPos::new(10, 20);
        assert_eq!(c.x(), 10);
        assert_eq!(c.y(), 20);
        assert_eq!(c.layer(), 0);
    }

    #[test]
    fn cpos_negative() {
        let c = CPos::new(-5, -10);
        assert_eq!(c.x(), -5);
        assert_eq!(c.y(), -10);
    }

    #[test]
    fn cpos_with_layer() {
        let c = CPos::with_layer(3, 7, 2);
        assert_eq!(c.x(), 3);
        assert_eq!(c.y(), 7);
        assert_eq!(c.layer(), 2);
    }

    #[test]
    fn cpos_from_bits_roundtrip() {
        // Test that from_bits(bits) preserves the original value
        let original = CPos::new(42, -15);
        let reconstructed = CPos::from_bits(original.bits);
        assert_eq!(reconstructed.x(), 42);
        assert_eq!(reconstructed.y(), -15);
    }

    #[test]
    fn cpos_zero() {
        assert_eq!(CPos::ZERO.x(), 0);
        assert_eq!(CPos::ZERO.y(), 0);
        assert_eq!(CPos::ZERO.layer(), 0);
        assert_eq!(CPos::ZERO.bits, 0);
    }

    #[test]
    fn cpos_hash_is_bits() {
        let c = CPos::new(10, 20);
        assert_eq!(c.sync_hash(), c.bits);
    }
}
