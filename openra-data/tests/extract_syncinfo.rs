//! Temporary test to extract SyncInfo from replay

use openra_data::orarep;

#[test]
fn extract_syncinfo() {
    let data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).unwrap();
    let replay = orarep::parse(&data).unwrap();

    for (frame, order) in &replay.orders {
        if order.order_string == "SyncInfo" || order.order_string == "SyncLobbyClients" {
            eprintln!("=== {} at frame {} ===", order.order_string, frame);
            if let Some(ref ts) = order.target_string {
                eprintln!("{}", ts);
            }
        }
    }
}
