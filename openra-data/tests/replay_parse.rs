//! Integration test: parse a real .orarep file.

use openra_data::orarep;

#[test]
fn parse_real_replay() {
    let data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).expect("Failed to read test replay file");

    let replay = orarep::parse(&data).expect("Failed to parse replay");

    // Basic sanity checks
    assert!(replay.tick_count > 0, "tick_count should be > 0, got {}", replay.tick_count);
    assert!(!replay.packets.is_empty(), "should have packets");
    assert!(!replay.orders.is_empty(), "should have parsed orders");

    // Print summary for debugging
    eprintln!("=== Replay Summary ===");
    eprintln!("Tick count: {}", replay.tick_count);
    eprintln!("Total packets: {}", replay.packets.len());
    eprintln!("Total orders: {}", replay.orders.len());

    if let Some(ref yaml) = replay.metadata_yaml {
        eprintln!("Metadata YAML length: {} bytes", yaml.len());
        // Print first few lines of metadata
        for line in yaml.lines().take(10) {
            eprintln!("  {}", line);
        }
    } else {
        eprintln!("No metadata found");
    }

    // Count order types
    let mut order_types: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, order) in &replay.orders {
        *order_types.entry(order.order_string.clone()).or_default() += 1;
    }
    eprintln!("\nOrder types:");
    let mut sorted: Vec<_> = order_types.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in sorted {
        eprintln!("  {}: {}", name, count);
    }
}
