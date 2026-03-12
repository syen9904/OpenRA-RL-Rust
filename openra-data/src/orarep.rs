//! `.orarep` replay file parser.
//!
//! Binary format: sequence of (ClientID: i32, PacketLength: i32, PacketData: [u8])
//! followed by metadata section (MetaStartMarker=-1, version, YAML, length, MetaEndMarker=-2).
//!
//! Reference: OpenRA.Game/Network/ReplayConnection.cs, Order.cs, OrderIO.cs

use std::io::{self, Cursor, Read};

/// Marker values for metadata section
const META_START_MARKER: i32 = -1;
const META_END_MARKER: i32 = -2;

/// Order type byte values from OrderType enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderType {
    Ack = 0x10,
    Ping = 0x20,
    SyncHash = 0x65,
    TickScale = 0x76,
    Disconnect = 0xBF,
    Handshake = 0xFE,
    Fields = 0xFF,
}

impl OrderType {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x10 => Some(Self::Ack),
            0x20 => Some(Self::Ping),
            0x65 => Some(Self::SyncHash),
            0x76 => Some(Self::TickScale),
            0xBF => Some(Self::Disconnect),
            0xFE => Some(Self::Handshake),
            0xFF => Some(Self::Fields),
            _ => None,
        }
    }
}

bitflags::bitflags! {
    /// Order field flags from OrderFields enum
    #[derive(Debug, Clone, Copy)]
    pub struct OrderFields: i16 {
        const TARGET        = 0x01;
        const EXTRA_ACTORS  = 0x02;
        const TARGET_STRING = 0x04;
        const QUEUED        = 0x08;
        const EXTRA_LOCATION = 0x10;
        const EXTRA_DATA    = 0x20;
        const TARGET_IS_CELL = 0x40;
        const SUBJECT       = 0x80;
        const GROUPED       = 0x100;
    }
}

/// Target type for order targets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetType {
    Actor,
    FrozenActor,
    Terrain,
}

/// Parsed target information
#[derive(Debug, Clone)]
pub enum OrderTarget {
    Invalid,
    Actor {
        actor_id: u32,
        generation: i32,
    },
    FrozenActor {
        player_actor_id: u32,
        frozen_actor_id: u32,
    },
    TerrainCell {
        cell_bits: i32,
        sub_cell: u8,
    },
    TerrainPos {
        pos: [i32; 3],
        terrain_positions: Vec<[i32; 3]>,
    },
}

/// A parsed order from the replay stream
#[derive(Debug, Clone)]
pub struct Order {
    pub order_type: OrderType,
    pub order_string: String,
    pub flags: OrderFields,
    pub subject_id: Option<u32>,
    pub target: OrderTarget,
    pub target_string: Option<String>,
    pub queued: bool,
    pub extra_actors: Vec<u32>,
    pub extra_location: Option<i32>,
    pub extra_data: Option<u32>,
    pub grouped_actors: Vec<u32>,
}

/// A raw packet from the replay file
#[derive(Debug, Clone)]
pub struct RawPacket {
    pub client_id: i32,
    pub frame: i32,
    pub data: Vec<u8>,
}

/// A SyncHash entry extracted from the replay
#[derive(Debug, Clone, Copy)]
pub struct SyncHashEntry {
    pub frame: i32,
    pub sync_hash: i32,
    pub defeat_state: u64,
}

/// Parsed replay file
#[derive(Debug)]
pub struct Replay {
    pub packets: Vec<RawPacket>,
    pub orders: Vec<(i32, Order)>, // (frame, order)
    pub sync_hashes: Vec<SyncHashEntry>,
    pub tick_count: i32,
    pub metadata_yaml: Option<String>,
}

/// Read a .NET BinaryReader length-prefixed string (7-bit encoded length).
fn read_dotnet_string(cursor: &mut Cursor<&[u8]>) -> io::Result<String> {
    // .NET BinaryReader.ReadString uses 7-bit encoded integer for length
    let mut len: u32 = 0;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        cursor.read_exact(&mut byte)?;
        len |= ((byte[0] & 0x7F) as u32) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Bad 7-bit encoded int"));
        }
    }
    let mut buf = vec![0u8; len as usize];
    cursor.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_i16(cursor: &mut Cursor<&[u8]>) -> io::Result<i16> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf)?;
    Ok(i16::from_le_bytes(buf))
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> io::Result<i32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf)?;
    Ok(buf[0])
}

/// Parse a single order from the binary stream.
/// Reference: Order.Deserialize() in Order.cs
fn parse_order(cursor: &mut Cursor<&[u8]>) -> io::Result<Option<Order>> {
    let type_byte = read_u8(cursor)?;
    let order_type = match OrderType::from_byte(type_byte) {
        Some(t) => t,
        None => return Ok(None), // Unknown order type, skip
    };

    match order_type {
        OrderType::Fields => {
            let order_string = read_dotnet_string(cursor)?;
            let flags = OrderFields::from_bits_truncate(read_i16(cursor)?);

            let subject_id = if flags.contains(OrderFields::SUBJECT) {
                Some(read_u32(cursor)?)
            } else {
                None
            };

            let target = if flags.contains(OrderFields::TARGET) {
                let target_type = read_u8(cursor)?;
                match target_type {
                    0 => {
                        // TargetType.Actor
                        let actor_id = read_u32(cursor)?;
                        let generation = read_i32(cursor)?;
                        OrderTarget::Actor { actor_id, generation }
                    }
                    1 => {
                        // TargetType.FrozenActor
                        let player_actor_id = read_u32(cursor)?;
                        let frozen_actor_id = read_u32(cursor)?;
                        OrderTarget::FrozenActor { player_actor_id, frozen_actor_id }
                    }
                    2 => {
                        // TargetType.Terrain
                        if flags.contains(OrderFields::TARGET_IS_CELL) {
                            let cell_bits = read_i32(cursor)?;
                            let sub_cell = read_u8(cursor)?;
                            OrderTarget::TerrainCell { cell_bits, sub_cell }
                        } else {
                            let x = read_i32(cursor)?;
                            let y = read_i32(cursor)?;
                            let z = read_i32(cursor)?;
                            let pos = [x, y, z];

                            let num_terrain = read_i16(cursor)?;
                            let terrain_positions = if num_terrain == -1 {
                                vec![pos]
                            } else {
                                let mut positions = Vec::with_capacity(num_terrain as usize);
                                for _ in 0..num_terrain {
                                    let tx = read_i32(cursor)?;
                                    let ty = read_i32(cursor)?;
                                    let tz = read_i32(cursor)?;
                                    positions.push([tx, ty, tz]);
                                }
                                positions
                            };
                            OrderTarget::TerrainPos { pos, terrain_positions }
                        }
                    }
                    _ => OrderTarget::Invalid,
                }
            } else {
                OrderTarget::Invalid
            };

            let target_string = if flags.contains(OrderFields::TARGET_STRING) {
                Some(read_dotnet_string(cursor)?)
            } else {
                None
            };

            let queued = flags.contains(OrderFields::QUEUED);

            let extra_actors = if flags.contains(OrderFields::EXTRA_ACTORS) {
                let count = read_i32(cursor)? as usize;
                let mut actors = Vec::with_capacity(count);
                for _ in 0..count {
                    actors.push(read_u32(cursor)?);
                }
                actors
            } else {
                Vec::new()
            };

            let extra_location = if flags.contains(OrderFields::EXTRA_LOCATION) {
                Some(read_i32(cursor)?)
            } else {
                None
            };

            let extra_data = if flags.contains(OrderFields::EXTRA_DATA) {
                Some(read_u32(cursor)?)
            } else {
                None
            };

            let grouped_actors = if flags.contains(OrderFields::GROUPED) {
                let count = read_i32(cursor)? as usize;
                let mut actors = Vec::with_capacity(count);
                for _ in 0..count {
                    actors.push(read_u32(cursor)?);
                }
                actors
            } else {
                Vec::new()
            };

            Ok(Some(Order {
                order_type,
                order_string,
                flags,
                subject_id,
                target,
                target_string,
                queued,
                extra_actors,
                extra_location,
                extra_data,
                grouped_actors,
            }))
        }

        OrderType::Handshake => {
            let name = read_dotnet_string(cursor)?;
            let target_string = read_dotnet_string(cursor)?;
            Ok(Some(Order {
                order_type,
                order_string: name,
                flags: OrderFields::empty(),
                subject_id: None,
                target: OrderTarget::Invalid,
                target_string: Some(target_string),
                queued: false,
                extra_actors: Vec::new(),
                extra_location: None,
                extra_data: None,
                grouped_actors: Vec::new(),
            }))
        }

        OrderType::SyncHash | OrderType::Disconnect | OrderType::Ack
        | OrderType::Ping | OrderType::TickScale => {
            // These are not game orders — skip them
            Ok(None)
        }
    }
}

/// Parse all orders from a packet's data (after the 4-byte frame number).
fn parse_orders_from_packet(data: &[u8]) -> Vec<Order> {
    if data.len() <= 4 {
        return Vec::new();
    }

    let order_data = &data[4..]; // Skip frame number
    let mut cursor = Cursor::new(order_data);
    let mut orders = Vec::new();

    while (cursor.position() as usize) < order_data.len() {
        match parse_order(&mut cursor) {
            Ok(Some(order)) => orders.push(order),
            Ok(None) => {} // Unknown or non-game order, skip
            Err(_) => break, // Parse error, stop
        }
    }

    orders
}

/// Parse metadata YAML from the end of the replay file.
fn parse_metadata(data: &[u8]) -> Option<String> {
    if data.len() < 16 {
        return None;
    }

    // Read end marker and data length from last 8 bytes
    let end_marker = i32::from_le_bytes(data[data.len() - 4..].try_into().ok()?);
    if end_marker != META_END_MARKER {
        return None;
    }

    let data_length = i32::from_le_bytes(data[data.len() - 8..data.len() - 4].try_into().ok()?) as usize;

    // Go back to find start marker: end - 4(end_marker) - 4(data_length) - data_length - 4(version) - 4(start_marker)
    let meta_start = data.len().checked_sub(8 + data_length + 8)?;
    let start_marker = i32::from_le_bytes(data[meta_start..meta_start + 4].try_into().ok()?);
    if start_marker != META_START_MARKER {
        return None;
    }

    // Skip start marker (4) + version (4), read data_length bytes
    let yaml_start = meta_start + 8;
    let yaml_end = yaml_start + data_length;
    if yaml_end > data.len() - 8 {
        return None;
    }

    String::from_utf8(data[yaml_start..yaml_end].to_vec()).ok()
}

/// Parse a complete .orarep file from bytes.
pub fn parse(data: &[u8]) -> io::Result<Replay> {
    let mut cursor = Cursor::new(data);
    let mut packets = Vec::new();
    let mut orders = Vec::new();
    let mut sync_hashes = Vec::new();
    let mut tick_count: i32 = 0;

    let data_len = data.len() as u64;

    while cursor.position() + 8 <= data_len {
        let client_id = read_i32(&mut cursor)?;
        if client_id == META_START_MARKER {
            break;
        }

        let packet_len = read_i32(&mut cursor)?;
        if packet_len < 0 || cursor.position() + packet_len as u64 > data_len {
            break;
        }

        let mut packet_data = vec![0u8; packet_len as usize];
        cursor.read_exact(&mut packet_data)?;

        if packet_data.len() < 4 {
            continue;
        }

        let frame = i32::from_le_bytes(packet_data[0..4].try_into().unwrap());

        // Extract SyncHash packets, skip Disconnect
        if packet_data.len() > 4 {
            let order_type = packet_data[4];
            if order_type == OrderType::SyncHash as u8 && packet_data.len() >= 17 {
                sync_hashes.push(SyncHashEntry {
                    frame,
                    sync_hash: i32::from_le_bytes(packet_data[5..9].try_into().unwrap()),
                    defeat_state: u64::from_le_bytes(packet_data[9..17].try_into().unwrap()),
                });
                packets.push(RawPacket { client_id, frame, data: packet_data });
                continue;
            }
            if order_type == OrderType::Disconnect as u8 {
                packets.push(RawPacket { client_id, frame, data: packet_data });
                continue;
            }
        }

        // Parse orders from this packet
        let packet_orders = parse_orders_from_packet(&packet_data);
        for order in packet_orders {
            orders.push((frame, order));
        }

        if frame > 0 {
            tick_count = tick_count.max(frame);
        }

        packets.push(RawPacket {
            client_id,
            frame,
            data: packet_data,
        });
    }

    let metadata_yaml = parse_metadata(data);

    Ok(Replay {
        packets,
        orders,
        sync_hashes,
        tick_count,
        metadata_yaml,
    })
}

/// Lobby settings extracted from the replay's SyncInfo.
#[derive(Debug, Clone)]
pub struct LobbySettings {
    pub random_seed: i32,
    pub starting_cash: i32,
    pub allow_spectators: bool,
    /// Occupied slots in order: (slot_name, player_reference, client_faction)
    pub occupied_slots: Vec<(String, String, String)>,
}

impl Replay {
    /// Extract the RandomSeed from the SyncInfo lobby order.
    pub fn random_seed(&self) -> Option<i32> {
        self.lobby_settings().map(|s| s.random_seed)
    }

    /// Extract the last (most complete) SyncInfo from the replay.
    fn last_sync_info(&self) -> Option<&str> {
        let mut last = None;
        for (_frame, order) in &self.orders {
            if order.order_string == "SyncInfo" {
                if let Some(ref ts) = order.target_string {
                    last = Some(ts.as_str());
                }
            }
        }
        last
    }

    /// Parse full lobby settings from the replay's SyncInfo orders.
    pub fn lobby_settings(&self) -> Option<LobbySettings> {
        let yaml = self.last_sync_info()?;
        let lines: Vec<&str> = yaml.lines().collect();

        let mut random_seed = 0i32;
        let mut starting_cash = 5000i32;
        let mut allow_spectators = true;

        // Parse GlobalSettings
        let mut in_startingcash = false;
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with("RandomSeed:") {
                random_seed = trimmed["RandomSeed:".len()..].trim().parse().ok()?;
            } else if trimmed.starts_with("AllowSpectators:") {
                allow_spectators = trimmed["AllowSpectators:".len()..].trim() == "True";
            } else if trimmed == "startingcash:" {
                in_startingcash = true;
                continue;
            }
            if in_startingcash && trimmed.starts_with("Value:") {
                starting_cash = trimmed["Value:".len()..].trim().parse().unwrap_or(5000);
                in_startingcash = false;
            }
        }

        // Parse slot definitions in order
        let mut slots: Vec<(String, String)> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("Slot@") && trimmed.ends_with(':') {
                let slot_name = &trimmed["Slot@".len()..trimmed.len() - 1];
                let mut player_ref = slot_name.to_string();
                for j in (i + 1)..lines.len() {
                    let inner = lines[j].trim();
                    if inner.starts_with("PlayerReference:") {
                        player_ref = inner["PlayerReference:".len()..].trim().to_string();
                        break;
                    }
                    if !lines[j].starts_with('\t') && !lines[j].starts_with("  ") {
                        break;
                    }
                }
                slots.push((slot_name.to_string(), player_ref));
            }
        }

        // Parse clients with their slot assignments
        let mut client_slots: Vec<(String, String)> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("Client@") && trimmed.ends_with(':') {
                let mut slot = String::new();
                let mut faction = String::from("Random");
                for j in (i + 1)..lines.len() {
                    let inner = lines[j].trim();
                    if inner.starts_with("Slot:") {
                        slot = inner["Slot:".len()..].trim().to_string();
                    } else if inner.starts_with("Faction:") {
                        faction = inner["Faction:".len()..].trim().to_string();
                    }
                    if !lines[j].starts_with('\t') && !lines[j].starts_with("  ") {
                        break;
                    }
                }
                if !slot.is_empty() {
                    client_slots.push((slot, faction));
                }
            }
        }

        // Match slots to clients in slot definition order
        let mut occupied = Vec::new();
        for (slot_name, player_ref) in &slots {
            if let Some((_, faction)) = client_slots.iter().find(|(s, _)| s == slot_name) {
                occupied.push((slot_name.clone(), player_ref.clone(), faction.clone()));
            }
        }

        Some(LobbySettings {
            random_seed,
            starting_cash,
            allow_spectators,
            occupied_slots: occupied,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_string_simple() {
        // Length=5, "hello"
        let data = [5, b'h', b'e', b'l', b'l', b'o'];
        let mut cursor = Cursor::new(data.as_slice());
        let s = read_dotnet_string(&mut cursor).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn dotnet_string_empty() {
        let data = [0u8];
        let mut cursor = Cursor::new(data.as_slice());
        let s = read_dotnet_string(&mut cursor).unwrap();
        assert_eq!(s, "");
    }

    #[test]
    fn dotnet_string_7bit_long() {
        // Length=128 uses 2 bytes: 0x80, 0x01
        let mut data = vec![0x80, 0x01];
        data.extend(vec![b'A'; 128]);
        let mut cursor = Cursor::new(data.as_slice());
        let s = read_dotnet_string(&mut cursor).unwrap();
        assert_eq!(s.len(), 128);
        assert!(s.chars().all(|c| c == 'A'));
    }

    #[test]
    fn parse_empty_packet_stream() {
        // Just a metadata start marker
        let mut data = Vec::new();
        data.extend_from_slice(&META_START_MARKER.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes()); // fake packet len (won't be read)
        let replay = parse(&data).unwrap();
        assert_eq!(replay.packets.len(), 0);
        assert_eq!(replay.tick_count, 0);
    }

    #[test]
    fn parse_metadata_valid() {
        let yaml = b"Root:\n  Key: Value\n";
        let mut data = Vec::new();

        // Start marker
        data.extend_from_slice(&META_START_MARKER.to_le_bytes());
        // Version
        data.extend_from_slice(&1i32.to_le_bytes());
        // YAML data
        data.extend_from_slice(yaml);
        // Data length
        data.extend_from_slice(&(yaml.len() as i32).to_le_bytes());
        // End marker
        data.extend_from_slice(&META_END_MARKER.to_le_bytes());

        let result = parse_metadata(&data);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Key: Value"));
    }
}
