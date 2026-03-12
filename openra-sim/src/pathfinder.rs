//! A* pathfinding on the terrain grid.
//!
//! 8-directional movement with terrain cost weighting.
//! Deterministic: consistent tie-breaking by cell position.

use crate::terrain::TerrainMap;
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// Orthogonal movement cost multiplier (×100 for fixed-point).
const ORTHO_COST: i32 = 100;
/// Diagonal movement cost multiplier (≈141, √2 × 100).
const DIAG_COST: i32 = 141;

/// 8-directional neighbor offsets: N, NE, E, SE, S, SW, W, NW.
const DIRS: [(i32, i32); 8] = [
    (0, -1), (1, -1), (1, 0), (1, 1),
    (0, 1), (-1, 1), (-1, 0), (-1, -1),
];

/// A* node in the priority queue.
#[derive(Debug, Clone)]
struct Node {
    x: i32,
    y: i32,
    g: i32,
    f: i32,
}

impl Eq for Node {}
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.g == other.g
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: reverse comparison. Tie-break by lower g (prefer explored).
        other.f.cmp(&self.f)
            .then_with(|| other.g.cmp(&self.g))
            .then_with(|| self.y.cmp(&other.y))
            .then_with(|| self.x.cmp(&other.x))
    }
}

/// Diagonal distance heuristic (admissible for 8-directional movement).
fn heuristic(x: i32, y: i32, gx: i32, gy: i32) -> i32 {
    let dx = (x - gx).abs();
    let dy = (y - gy).abs();
    let diag = dx.min(dy);
    let straight = dx.max(dy) - diag;
    straight * ORTHO_COST + diag * DIAG_COST
}

/// Find a path from `from` to `to` on the terrain map.
///
/// Returns the path as a list of cells from start to goal (inclusive),
/// or None if no path exists.
///
/// `ignore_actor` optionally allows pathing through cells occupied by
/// a specific actor (e.g., the moving unit itself).
pub fn find_path(
    terrain: &TerrainMap,
    from: (i32, i32),
    to: (i32, i32),
    ignore_actor: Option<u32>,
) -> Option<Vec<(i32, i32)>> {
    if from == to {
        return Some(vec![from]);
    }

    let w = terrain.width;
    let h = terrain.height;
    let size = (w * h) as usize;

    // g-cost and parent arrays
    let mut g_cost = vec![i32::MAX; size];
    let mut parent = vec![(-1i32, -1i32); size];
    let mut closed = vec![false; size];

    let idx = |x: i32, y: i32| -> usize { (y * w + x) as usize };

    let mut open = BinaryHeap::new();

    g_cost[idx(from.0, from.1)] = 0;
    open.push(Node {
        x: from.0,
        y: from.1,
        g: 0,
        f: heuristic(from.0, from.1, to.0, to.1),
    });

    while let Some(current) = open.pop() {
        let ci = idx(current.x, current.y);
        if closed[ci] {
            continue;
        }
        closed[ci] = true;

        if (current.x, current.y) == to {
            // Reconstruct path
            let mut path = Vec::new();
            let mut pos = to;
            while pos != from {
                path.push(pos);
                pos = parent[idx(pos.0, pos.1)];
            }
            path.push(from);
            path.reverse();
            return Some(path);
        }

        for &(dx, dy) in &DIRS {
            let nx = current.x + dx;
            let ny = current.y + dy;

            if !terrain.contains(nx, ny) {
                continue;
            }

            let ni = idx(nx, ny);
            if closed[ni] {
                continue;
            }

            // Check passability
            let terrain_cost = terrain.cost(nx, ny);
            if terrain_cost == crate::terrain::COST_IMPASSABLE {
                continue;
            }

            // Allow pathing through cells occupied by the moving unit itself
            let occupant = terrain.occupant(nx, ny);
            if occupant != 0 {
                if let Some(ignore) = ignore_actor {
                    if occupant != ignore && (nx, ny) != to {
                        continue;
                    }
                } else if (nx, ny) != to {
                    continue;
                }
            }

            // Movement cost: terrain cost × direction multiplier
            let is_diag = dx != 0 && dy != 0;
            let move_cost = terrain_cost as i32 * if is_diag { DIAG_COST } else { ORTHO_COST } / 100;
            let new_g = current.g + move_cost;

            if new_g < g_cost[ni] {
                g_cost[ni] = new_g;
                parent[ni] = (current.x, current.y);
                let h = heuristic(nx, ny, to.0, to.1);
                open.push(Node { x: nx, y: ny, g: new_g, f: new_g + h });
            }
        }
    }

    None // No path found
}

/// Compute the WAngle facing from cell `from` to cell `to`.
///
/// OpenRA uses 0-1023 angles: 0=North, 256=East, 512=South, 768=West.
pub fn facing_between(from: (i32, i32), to: (i32, i32)) -> i32 {
    let dx = (to.0 - from.0).signum();
    let dy = (to.1 - from.1).signum();
    match (dx, dy) {
        (0, -1) => 0,     // North
        (1, -1) => 128,   // NE
        (1, 0) => 256,    // East
        (1, 1) => 384,    // SE
        (0, 1) => 512,    // South
        (-1, 1) => 640,   // SW
        (-1, 0) => 768,   // West
        (-1, -1) => 896,  // NW
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_straight_line() {
        let terrain = TerrainMap::new(10, 10);
        let path = find_path(&terrain, (0, 0), (5, 0), None).unwrap();
        assert_eq!(path.first(), Some(&(0, 0)));
        assert_eq!(path.last(), Some(&(5, 0)));
        assert!(path.len() <= 6); // Should be 6 cells (inclusive)
    }

    #[test]
    fn path_diagonal() {
        let terrain = TerrainMap::new(10, 10);
        let path = find_path(&terrain, (0, 0), (5, 5), None).unwrap();
        assert_eq!(path.first(), Some(&(0, 0)));
        assert_eq!(path.last(), Some(&(5, 5)));
        // Diagonal path should be ~6 steps
        assert!(path.len() <= 7);
    }

    #[test]
    fn path_around_obstacle() {
        let mut terrain = TerrainMap::new(10, 10);
        // Wall from (3,0) to (3,8)
        for y in 0..9 {
            terrain.set_cost(3, y, crate::terrain::COST_IMPASSABLE);
        }
        let path = find_path(&terrain, (1, 5), (5, 5), None).unwrap();
        assert_eq!(path.last(), Some(&(5, 5)));
        // Should go around the wall (through (3,9))
        assert!(path.iter().all(|&(x, y)| terrain.is_terrain_passable(x, y)));
    }

    #[test]
    fn path_blocked() {
        let mut terrain = TerrainMap::new(5, 5);
        // Surround (4,4) with impassable
        for x in 3..5 { terrain.set_cost(x, 3, crate::terrain::COST_IMPASSABLE); }
        terrain.set_cost(3, 4, crate::terrain::COST_IMPASSABLE);
        let result = find_path(&terrain, (0, 0), (4, 4), None);
        assert!(result.is_none());
    }

    #[test]
    fn path_same_cell() {
        let terrain = TerrainMap::new(10, 10);
        let path = find_path(&terrain, (5, 5), (5, 5), None).unwrap();
        assert_eq!(path, vec![(5, 5)]);
    }

    #[test]
    fn facing_directions() {
        assert_eq!(facing_between((5, 5), (5, 4)), 0);    // North
        assert_eq!(facing_between((5, 5), (6, 4)), 128);  // NE
        assert_eq!(facing_between((5, 5), (6, 5)), 256);  // East
        assert_eq!(facing_between((5, 5), (6, 6)), 384);  // SE
        assert_eq!(facing_between((5, 5), (5, 6)), 512);  // South
        assert_eq!(facing_between((5, 5), (4, 6)), 640);  // SW
        assert_eq!(facing_between((5, 5), (4, 5)), 768);  // West
        assert_eq!(facing_between((5, 5), (4, 4)), 896);  // NW
    }
}
