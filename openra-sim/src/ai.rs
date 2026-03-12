//! Simple bot AI for Red Alert.
//!
//! Generates orders each tick: builds power, barracks, war factory,
//! produces units, sends them to attack. Roughly based on OpenRA's
//! HackyAI bot modules.

use crate::actor::ActorKind;
use crate::world::{GameOrder, World};

/// AI state for one bot player.
#[derive(Debug)]
pub struct Bot {
    /// The player actor ID this bot controls.
    pub player_id: u32,
    /// Ticks since last action.
    ticks_idle: u32,
    /// Current strategic state.
    state: BotState,
    /// Whether we've placed our first power plant.
    has_power: bool,
    /// Whether we've queued barracks.
    has_barracks: bool,
    /// Whether we've queued war factory.
    has_war_factory: bool,
    /// Whether we've queued refinery.
    has_refinery: bool,
    /// Number of units produced.
    units_produced: u32,
    /// Attack wave threshold.
    attack_threshold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BotState {
    /// Building up base infrastructure.
    BuildUp,
    /// Producing army units.
    Producing,
    /// Sending units to attack.
    Attacking,
}

impl Bot {
    pub fn new(player_id: u32) -> Self {
        Bot {
            player_id,
            ticks_idle: 0,
            state: BotState::BuildUp,
            has_power: false,
            has_barracks: false,
            has_war_factory: false,
            has_refinery: false,
            units_produced: 0,
            attack_threshold: 5,
        }
    }

    /// Generate orders for this tick.
    pub fn tick(&mut self, world: &World) -> Vec<GameOrder> {
        self.ticks_idle += 1;
        let mut orders = Vec::new();

        // Only act every ~30 ticks to avoid spamming
        if self.ticks_idle < 30 {
            return orders;
        }
        self.ticks_idle = 0;

        match self.state {
            BotState::BuildUp => {
                self.do_build_up(world, &mut orders);
            }
            BotState::Producing => {
                self.do_produce(world, &mut orders);
            }
            BotState::Attacking => {
                self.do_attack(world, &mut orders);
            }
        }

        orders
    }

    fn do_build_up(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);

        // Check what buildings we have
        self.survey_buildings(world);

        if !self.has_power && cash >= 300 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("powr".to_string()),
                extra_data: None,
            });
            self.has_power = true;
            return;
        }

        if self.has_power && !self.has_barracks && cash >= 400 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("barr".to_string()),
                extra_data: None,
            });
            self.has_barracks = true;
            return;
        }

        if self.has_barracks && !self.has_refinery && cash >= 1400 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("proc".to_string()),
                extra_data: None,
            });
            self.has_refinery = true;
            return;
        }

        if self.has_refinery && !self.has_war_factory && cash >= 2000 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("weap".to_string()),
                extra_data: None,
            });
            self.has_war_factory = true;
            return;
        }

        // Once we have basic infrastructure, switch to producing
        if self.has_power && self.has_barracks {
            self.state = BotState::Producing;
        }
    }

    fn do_produce(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);

        // Produce a mix of infantry and vehicles
        if self.has_war_factory && cash >= 800 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("2tnk".to_string()),
                extra_data: None,
            });
            self.units_produced += 1;
        } else if cash >= 100 {
            orders.push(GameOrder {
                order_string: "StartProduction".to_string(),
                subject_id: Some(self.player_id),
                target_string: Some("e1".to_string()),
                extra_data: None,
            });
            self.units_produced += 1;
        }

        // Once we have enough units, switch to attacking
        if self.units_produced >= self.attack_threshold {
            self.state = BotState::Attacking;
        }
    }

    fn do_attack(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Find enemy actors to attack
        let enemy_target = self.find_enemy_target(world);
        if let Some((target_x, target_y)) = enemy_target {
            // Send all our military units to attack
            let our_units: Vec<u32> = world.actor_ids_for_player(self.player_id)
                .into_iter()
                .filter(|&id| {
                    world.actor_kind(id).map(|k| {
                        matches!(k, ActorKind::Infantry | ActorKind::Vehicle)
                    }).unwrap_or(false)
                })
                .collect();

            for unit_id in our_units {
                orders.push(GameOrder {
                    order_string: "AttackMove".to_string(),
                    subject_id: Some(unit_id),
                    target_string: Some(format!("{},{}", target_x, target_y)),
                    extra_data: None,
                });
            }
        }

        // Go back to producing after attack
        self.units_produced = 0;
        self.attack_threshold += 2; // Increase threshold each wave
        self.state = BotState::Producing;
    }

    fn player_cash(&self, world: &World) -> i32 {
        world.player_cash(self.player_id)
    }

    fn survey_buildings(&mut self, world: &World) {
        let building_types = world.player_building_types(self.player_id);
        self.has_power = building_types.iter().any(|t| t == "powr" || t == "apwr");
        self.has_barracks = building_types.iter().any(|t| t == "tent" || t == "barr");
        self.has_war_factory = building_types.iter().any(|t| t == "weap" || t == "weap.ukraine");
        self.has_refinery = building_types.iter().any(|t| t == "proc");
    }

    fn find_enemy_target(&self, world: &World) -> Option<(i32, i32)> {
        // Find any enemy unit or building location
        world.find_enemy_location(self.player_id)
    }
}
