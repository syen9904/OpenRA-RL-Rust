//! Terrain layer — cell grid, movement costs, and occupancy.
//!
//! Provides `CellLayer<T>` for 2D grid storage and `TerrainMap`
//! for querying movement costs and building/actor occupancy.

/// A 2D grid layer indexed by cell position (x, y).
///
/// Uses rectangular grid layout matching RA mod.
/// Index = y * width + x.
#[derive(Debug, Clone)]
pub struct CellLayer<T> {
    pub width: i32,
    pub height: i32,
    data: Vec<T>,
}

impl<T: Default + Clone> CellLayer<T> {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            data: vec![T::default(); (width * height) as usize],
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    pub fn get(&self, x: i32, y: i32) -> &T {
        &self.data[self.index(x, y)]
    }

    pub fn set(&mut self, x: i32, y: i32, val: T) {
        let idx = self.index(x, y);
        self.data[idx] = val;
    }
}

/// Movement cost for a cell. 0 = impassable, 100 = normal, >100 = rough.
pub const COST_IMPASSABLE: i16 = i16::MAX;
pub const COST_NORMAL: i16 = 100;

/// Resource type in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceType {
    #[default]
    None,
    Ore,
    Gems,
}

/// Resource contents of a cell: type + density (0-12 for ore, 0-3 for gems).
#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceCell {
    pub resource_type: ResourceType,
    pub density: u8,
}

/// The world's terrain and occupancy state.
#[derive(Debug, Clone)]
pub struct TerrainMap {
    pub width: i32,
    pub height: i32,
    /// Base terrain movement cost per cell.
    costs: CellLayer<i16>,
    /// Actor ID occupying each cell (0 = empty). For buildings with footprints,
    /// all footprint cells store the building's actor ID.
    occupancy: CellLayer<u32>,
    /// Resource layer: ore/gems per cell.
    resources: CellLayer<ResourceCell>,
}

impl TerrainMap {
    /// Create terrain map from map dimensions.
    /// All cells default to normal passable terrain.
    pub fn new(width: i32, height: i32) -> Self {
        let mut costs = CellLayer::new(width, height);
        for y in 0..height {
            for x in 0..width {
                costs.set(x, y, COST_NORMAL);
            }
        }
        Self {
            width,
            height,
            costs,
            occupancy: CellLayer::new(width, height),
            resources: CellLayer::new(width, height),
        }
    }

    /// Check if a cell is within map bounds.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.costs.contains(x, y)
    }

    /// Get the base movement cost for a cell.
    pub fn cost(&self, x: i32, y: i32) -> i16 {
        if !self.contains(x, y) {
            return COST_IMPASSABLE;
        }
        *self.costs.get(x, y)
    }

    /// Check if a cell is passable (not impassable terrain AND not occupied).
    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        self.contains(x, y)
            && *self.costs.get(x, y) != COST_IMPASSABLE
            && *self.occupancy.get(x, y) == 0
    }

    /// Check if a cell is passable for a specific actor (passable if empty or self).
    pub fn is_passable_for(&self, x: i32, y: i32, actor_id: u32) -> bool {
        self.contains(x, y)
            && *self.costs.get(x, y) != COST_IMPASSABLE
            && (*self.occupancy.get(x, y) == 0 || *self.occupancy.get(x, y) == actor_id)
    }

    /// Check if a cell is passable terrain (ignoring occupancy).
    pub fn is_terrain_passable(&self, x: i32, y: i32) -> bool {
        self.contains(x, y) && *self.costs.get(x, y) != COST_IMPASSABLE
    }

    /// Get the actor ID occupying a cell (0 = empty).
    pub fn occupant(&self, x: i32, y: i32) -> u32 {
        if !self.contains(x, y) { return 0; }
        *self.occupancy.get(x, y)
    }

    /// Mark a cell as occupied by an actor.
    pub fn set_occupant(&mut self, x: i32, y: i32, actor_id: u32) {
        if self.contains(x, y) {
            self.occupancy.set(x, y, actor_id);
        }
    }

    /// Clear occupancy for a cell.
    pub fn clear_occupant(&mut self, x: i32, y: i32) {
        if self.contains(x, y) {
            self.occupancy.set(x, y, 0);
        }
    }

    /// Occupy a rectangular footprint for a building.
    pub fn occupy_footprint(&mut self, top_left_x: i32, top_left_y: i32, w: i32, h: i32, actor_id: u32) {
        for dy in 0..h {
            for dx in 0..w {
                self.set_occupant(top_left_x + dx, top_left_y + dy, actor_id);
            }
        }
    }

    /// Clear a rectangular footprint.
    pub fn clear_footprint(&mut self, top_left_x: i32, top_left_y: i32, w: i32, h: i32) {
        for dy in 0..h {
            for dx in 0..w {
                self.clear_occupant(top_left_x + dx, top_left_y + dy);
            }
        }
    }

    /// Set terrain cost for a cell.
    pub fn set_cost(&mut self, x: i32, y: i32, cost: i16) {
        if self.contains(x, y) {
            self.costs.set(x, y, cost);
        }
    }

    /// Get the resource at a cell.
    pub fn resource(&self, x: i32, y: i32) -> ResourceCell {
        if !self.contains(x, y) { return ResourceCell::default(); }
        *self.resources.get(x, y)
    }

    /// Set resource at a cell.
    pub fn set_resource(&mut self, x: i32, y: i32, resource_type: ResourceType, density: u8) {
        if self.contains(x, y) {
            self.resources.set(x, y, ResourceCell { resource_type, density });
        }
    }

    /// Remove one unit of resource from a cell. Returns true if resource was removed.
    pub fn harvest_resource(&mut self, x: i32, y: i32) -> Option<ResourceType> {
        if !self.contains(x, y) { return None; }
        let cell = *self.resources.get(x, y);
        if cell.resource_type == ResourceType::None || cell.density == 0 {
            return None;
        }
        let new_density = cell.density - 1;
        if new_density == 0 {
            self.resources.set(x, y, ResourceCell::default());
        } else {
            self.resources.set(x, y, ResourceCell { resource_type: cell.resource_type, density: new_density });
        }
        Some(cell.resource_type)
    }

    /// Check if a cell has harvestable resources.
    pub fn has_resource(&self, x: i32, y: i32) -> bool {
        if !self.contains(x, y) { return false; }
        let cell = self.resources.get(x, y);
        cell.resource_type != ResourceType::None && cell.density > 0
    }

    /// Get total resource count (for snapshot summaries).
    pub fn total_resources(&self) -> i32 {
        let mut total = 0i32;
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = self.resources.get(x, y);
                total += cell.density as i32;
            }
        }
        total
    }

    /// Find the nearest cell with resources within a search radius.
    pub fn find_nearest_resource(&self, cx: i32, cy: i32, radius: i32) -> Option<(i32, i32)> {
        let mut best: Option<(i32, i32)> = None;
        let mut best_dist = i32::MAX;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let x = cx + dx;
                let y = cy + dy;
                if self.has_resource(x, y) {
                    let dist = dx.abs() + dy.abs(); // Manhattan distance
                    if dist < best_dist {
                        best_dist = dist;
                        best = Some((x, y));
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_layer_basic() {
        let mut layer: CellLayer<i32> = CellLayer::new(10, 10);
        assert_eq!(*layer.get(5, 5), 0);
        layer.set(5, 5, 42);
        assert_eq!(*layer.get(5, 5), 42);
    }

    #[test]
    fn cell_layer_bounds() {
        let layer: CellLayer<i32> = CellLayer::new(10, 10);
        assert!(layer.contains(0, 0));
        assert!(layer.contains(9, 9));
        assert!(!layer.contains(-1, 0));
        assert!(!layer.contains(10, 0));
    }

    #[test]
    fn terrain_map_passability() {
        let mut map = TerrainMap::new(10, 10);
        assert!(map.is_passable(5, 5));

        map.set_cost(5, 5, COST_IMPASSABLE);
        assert!(!map.is_passable(5, 5));
    }

    #[test]
    fn terrain_map_occupancy() {
        let mut map = TerrainMap::new(10, 10);
        assert!(map.is_passable(5, 5));

        map.set_occupant(5, 5, 100);
        assert!(!map.is_passable(5, 5));
        assert!(map.is_terrain_passable(5, 5));
        assert_eq!(map.occupant(5, 5), 100);

        map.clear_occupant(5, 5);
        assert!(map.is_passable(5, 5));
    }

    #[test]
    fn resource_layer() {
        let mut map = TerrainMap::new(20, 20);
        assert!(!map.has_resource(5, 5));

        map.set_resource(5, 5, ResourceType::Ore, 12);
        assert!(map.has_resource(5, 5));
        assert_eq!(map.resource(5, 5).density, 12);

        // Harvest one unit
        let rt = map.harvest_resource(5, 5);
        assert_eq!(rt, Some(ResourceType::Ore));
        assert_eq!(map.resource(5, 5).density, 11);

        // Find nearest resource
        map.set_resource(10, 10, ResourceType::Gems, 3);
        let nearest = map.find_nearest_resource(6, 6, 10);
        assert_eq!(nearest, Some((5, 5))); // Closer
    }

    #[test]
    fn footprint_occupy() {
        let mut map = TerrainMap::new(10, 10);
        map.occupy_footprint(3, 3, 2, 2, 50);
        assert_eq!(map.occupant(3, 3), 50);
        assert_eq!(map.occupant(4, 4), 50);
        assert_eq!(map.occupant(5, 5), 0);
    }
}
