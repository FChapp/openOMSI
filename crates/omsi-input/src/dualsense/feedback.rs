//! What the bus says through a DualSense, frame by frame: its rumble, its adaptive triggers
//! and its lights. Pure: the game hands in what the bus does (`Bus`) and the wheels' jolt the
//! force feedback already works out, and gets the pad's whole `Output` back.
//!
//! - rumble: nothing that never stops - a pad that buzzes as long as the engine runs, or as
//!   long as the bus rolls, is soon a pad that is not felt - but what happens: a real jolt of
//!   the road (a pothole, a kerb), the engine on its governor (the red of the rev counter,
//!   bouncing as the fuel is cut), and small knocks: a door shutting, a gear going in, the
//!   indicator's relay, a crash;
//! - L2, the brake: free; on a bus with a retarder free over the retarder's travel and then a
//!   notch where the brakes begin; it knocks while the ABS works (or, on a bus that does not
//!   say, when braked hard at speed or while the wheels slide);
//! - R2, the throttle: free, and a wall while the stop brake or the parking brake holds the
//!   bus (OMSI's buses cut the throttle then);
//! - the light bar: the indicators blinking orange with the lamps, red pulsing for an
//!   electrical fault, green while the doors are open or the stop brake holds, red for the
//!   parking brake, warm white otherwise;
//! - the player lights: the direction, forward (the middle one), neutral (the two inner
//!   ones), reverse (the two outer ones); the microphone light: a stop requested.

use super::report::{MicLed, Output, Trigger};

/// What the bus does this frame, as far as the pad shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bus {
    /// The player is driving it (a view of the bus, the game not paused, the window
    /// focused): else the pad rests - no rumble, free triggers, a warm white light.
    pub driving: bool,
    /// Its electrics are on.
    pub power: bool,
    /// The indicator switch: 0 off, 1 left, 2 right, 3 hazard; and whether a lamp is lit
    /// (None: the script has no lamp the game knows, they blink at the usual pace).
    pub indicator: u8,
    pub indicator_lamp: Option<bool>,
    pub parking_brake: bool,
    pub stop_brake: bool,
    pub doors_open: bool,
    pub electrical_fault: bool,
    pub stop_requested: bool,
    pub reverse: bool,
    /// The gear engaged where the script says (0 neutral).
    pub gear: Option<f32>,
    /// The bus has a retarder (its first steps are on the brake pedal's first travel).
    pub retarder: bool,
    /// The brake pedal, 0..1, and the throttle.
    pub brake: f32,
    pub throttle: f32,
    /// The ABS at work, where the script says (None: it does not).
    pub abs: Option<bool>,
    /// The tyres slide (the body no longer holds the road), and the speed.
    pub sliding: bool,
    pub kmh: f32,
    /// The engine's speed (rpm, 0: stopped or not known), and where its governor cuts in
    /// where the bus's constants say (else learnt as it is driven: `Haptics`).
    pub rpm: f32,
    pub redline: Option<f32>,
    /// Crashes so far and the energy of the latest (J).
    pub crashes: u32,
    pub last_impact: f32,
}

/// Jolts smaller than this are the road's own unevenness, not felt on the pad.
const BUMP_FLOOR: f32 = 0.15;
/// The engine buzzes from this share of its governor's speed, fully at the governor.
const RED_FROM: f32 = 0.94;

/// Warm white, the light bar at rest.
pub const WARM_WHITE: [u8; 3] = [255, 170, 90];
const ORANGE: [u8; 3] = [255, 80, 0];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 230, 40];

/// The pad's state over the frames: the knocks dying away, the governor's speed as learnt,
/// and the last frame's bus to tell what changed.
#[derive(Clone, Debug, Default)]
pub struct Haptics {
    t: f32,
    /// The knocks on the heavy (left) and the light (right) motor, 0..1, dying away.
    knock: [f32; 2],
    /// The governor's speed learnt: the engine at full throttle no longer rising (rpm), and
    /// the climb being watched (its peak, how long it has stayed there).
    governor: f32,
    climb: (f32, f32),
    last: Option<Bus>,
}

/// How fast a knock dies away (its strength a second).
const KNOCK_FADE: f32 = 8.0;

impl Haptics {
    /// The pad's output this frame. `bump`: the wheels' jolt as the force feedback holds it
    /// (0..1); `vibration`: the player's strength for the pad (0..2); `triggers`: for the
    /// triggers (0 off .. 1).
    pub fn frame(
        &mut self,
        bus: &Bus,
        bump: f32,
        vibration: f32,
        triggers: f32,
        dt: f32,
    ) -> Output {
        let dt = dt.clamp(0.0, 0.25);
        self.t = (self.t + dt) % 3600.0;
        for k in &mut self.knock {
            *k = (*k - KNOCK_FADE * dt).max(0.0);
        }
        if bus.driving {
            self.knocks(bus);
            self.learn_governor(bus, dt);
        }
        self.last = Some(*bus);
        if !bus.driving {
            self.knock = [0.0; 2];
            return Output {
                lightbar: WARM_WHITE,
                ..Output::default()
            };
        }

        let bump = ((bump - BUMP_FLOOR) / (1.0 - BUMP_FLOOR)).clamp(0.0, 1.0);
        let red = self.red(bus);
        let motor = |v: f32| ((v * vibration.clamp(0.0, 2.0)).clamp(0.0, 1.0) * 255.0) as u8;
        Output {
            rumble_left: motor(bump + red * 0.35 + self.knock[0]),
            rumble_right: motor(bump * 0.5 + red * 0.6 + self.knock[1]),
            trigger_left: scaled(brake_trigger(bus), triggers),
            trigger_right: scaled(throttle_trigger(bus), triggers),
            lightbar: self.lightbar(bus),
            player_leds: player_leds(bus),
            mic_led: if bus.stop_requested {
                MicLed::On
            } else {
                MicLed::Off
            },
            ..Output::default()
        }
    }

    /// The engine in the red: 0 below `RED_FROM` of the governor's speed, up to 1 at it, in
    /// the governor's bounces (the fuel cut and given again, some 12 times a second).
    fn red(&self, bus: &Bus) -> f32 {
        let Some(top) = bus
            .redline
            .or((self.governor > 0.0).then_some(self.governor))
        else {
            return 0.0;
        };
        let near = ((bus.rpm / top - RED_FROM) / (1.0 - RED_FROM)).clamp(0.0, 1.0);
        let bounce = if (self.t * 12.0).fract() < 0.5 {
            1.0
        } else {
            0.3
        };
        near * bounce
    }

    /// The governor's speed where the bus does not say it: the engine at full throttle, no
    /// longer rising for most of a second (as the automated manual learns it).
    fn learn_governor(&mut self, bus: &Bus, dt: f32) {
        let (peak, flat) = &mut self.climb;
        if bus.throttle < 0.8 || bus.rpm < 500.0 {
            self.climb = (0.0, 0.0);
        } else if bus.rpm > *peak * 1.01 {
            (*peak, *flat) = (bus.rpm, 0.0);
        } else {
            *flat += dt;
            if *flat > 0.8 {
                self.governor = self.governor.max(*peak);
            }
        }
    }

    /// The knocks of what changed since the last frame.
    fn knocks(&mut self, bus: &Bus) {
        let Some(last) = self.last.filter(|l| l.driving) else {
            return;
        };
        let mut knock = |motor: usize, v: f32| self.knock[motor] = self.knock[motor].max(v);
        // (small: a detail, not an event)
        if last.doors_open && !bus.doors_open {
            knock(0, 0.12);
            knock(1, 0.2);
        }
        if bus
            .gear
            .zip(last.gear)
            .is_some_and(|(a, b)| (a - b).abs() > 0.5)
        {
            knock(1, 0.25);
        }
        // the relay's click, as the lamps come on
        if bus.indicator != 0
            && bus.indicator_lamp == Some(true)
            && last.indicator_lamp == Some(false)
        {
            knock(1, 0.12);
        }
        if bus.crashes > last.crashes {
            // (a scrape some kJ, a lorry a hundred and more)
            let hard = (bus.last_impact / 60_000.0).clamp(0.35, 1.0);
            knock(0, hard);
            knock(1, hard * 0.7);
        }
    }

    fn lightbar(&self, bus: &Bus) -> [u8; 3] {
        let dim = |c: [u8; 3], k: f32| c.map(|v| (f32::from(v) * k) as u8);
        if bus.electrical_fault {
            let k = 0.55 + 0.45 * (self.t * std::f32::consts::TAU * 1.2).sin();
            return dim(RED, k);
        }
        if bus.indicator != 0 {
            // (the usual pace, 1.5 a second, where the script has no lamp the game knows)
            let lit = bus.indicator_lamp.unwrap_or((self.t * 1.5).fract() < 0.5);
            return if lit { ORANGE } else { dim(ORANGE, 0.08) };
        }
        if bus.doors_open || bus.stop_brake {
            return GREEN;
        }
        if bus.parking_brake {
            return RED;
        }
        if bus.power {
            WARM_WHITE
        } else {
            dim(WARM_WHITE, 0.25)
        }
    }
}

/// L2: the ABS's knocking while it works; else free, with the notch where the retarder's
/// travel ends on a bus that has one.
fn brake_trigger(bus: &Bus) -> Trigger {
    let abs = match bus.abs {
        Some(working) => working,
        // (a bus that does not say: braked hard at speed, or the wheels sliding)
        None => bus.kmh > 10.0 && (bus.brake > 0.85 || (bus.sliding && bus.brake > 0.2)),
    };
    if abs && bus.brake > 0.05 {
        // (slow enough to be felt as knocks, not as a hum)
        return Trigger::vibration(3, 8, 9);
    }
    if bus.retarder {
        // free over the retarder's travel (the first three zones), a step where the brakes
        // begin, and only a light spring after it
        Trigger::Feedback([0, 0, 0, 5, 2, 2, 2, 2, 2, 2])
    } else {
        Trigger::Off
    }
}

/// R2: a wall while the stop brake or the parking brake holds the bus, else free.
fn throttle_trigger(bus: &Bus) -> Trigger {
    if bus.stop_brake || bus.parking_brake {
        Trigger::resistance(0, 8)
    } else {
        Trigger::Off
    }
}

/// A trigger's effect at `k` (0..1) of its strength: none at 0.
fn scaled(t: Trigger, k: f32) -> Trigger {
    let k = k.clamp(0.0, 1.0);
    if k <= 0.01 {
        return Trigger::Off;
    }
    let s = |v: u8| {
        if v == 0 {
            0
        } else {
            ((f32::from(v) * k).round() as u8).clamp(1, 8)
        }
    };
    match t {
        Trigger::Feedback(z) => Trigger::Feedback(z.map(s)),
        Trigger::Vibration {
            amplitudes,
            frequency,
        } => Trigger::Vibration {
            amplitudes: amplitudes.map(s),
            frequency,
        },
        Trigger::Weapon {
            start,
            end,
            strength,
        } => Trigger::Weapon {
            start,
            end,
            strength: s(strength),
        },
        Trigger::Off => Trigger::Off,
    }
}

fn player_leds(bus: &Bus) -> u8 {
    match () {
        _ if !bus.power => 0,
        _ if bus.reverse => 0b10001,
        _ if bus.gear.is_some_and(|g| g.abs() < 0.5) => 0b01010,
        _ => 0b00100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driving() -> Bus {
        Bus {
            driving: true,
            power: true,
            gear: Some(1.0),
            ..Bus::default()
        }
    }

    fn frame(h: &mut Haptics, bus: &Bus, dt: f32) -> Output {
        h.frame(bus, 0.0, 1.0, 1.0, dt)
    }

    #[test]
    fn at_rest_the_pad_is_warm_white_and_still() {
        let mut h = Haptics::default();
        let out = h.frame(&Bus::default(), 1.0, 1.0, 1.0, 0.016);
        assert_eq!(out.lightbar, WARM_WHITE);
        assert_eq!((out.rumble_left, out.rumble_right), (0, 0));
        assert_eq!(
            (out.trigger_left, out.trigger_right),
            (Trigger::Off, Trigger::Off)
        );
    }

    #[test]
    fn an_engine_running_or_a_bus_rolling_is_not_felt() {
        let mut h = Haptics::default();
        let cruising = Bus {
            rpm: 1400.0,
            redline: Some(2300.0),
            kmh: 50.0,
            throttle: 0.4,
            ..driving()
        };
        for _ in 0..100 {
            let out = h.frame(&cruising, 0.1, 1.0, 1.0, 0.016);
            assert_eq!((out.rumble_left, out.rumble_right), (0, 0));
        }
    }

    #[test]
    fn only_real_jolts_of_the_road_are_felt() {
        let mut h = Haptics::default();
        assert_eq!(h.frame(&driving(), 0.1, 1.0, 1.0, 0.016).rumble_left, 0);
        assert!(h.frame(&driving(), 0.8, 1.0, 1.0, 0.016).rumble_left > 150);
        assert_eq!(h.frame(&driving(), 0.8, 0.0, 1.0, 0.016).rumble_left, 0);
    }

    #[test]
    fn the_engine_buzzes_in_the_red() {
        let mut h = Haptics::default();
        let red = Bus {
            rpm: 2300.0,
            redline: Some(2300.0),
            throttle: 1.0,
            ..driving()
        };
        let (mut strong, mut weak) = (0, 255);
        for _ in 0..30 {
            let r = frame(&mut h, &red, 0.016).rumble_right;
            strong = strong.max(r);
            weak = weak.min(r);
        }
        // (bouncing on the governor)
        assert!(strong > 140 && weak < strong / 2, "{strong} {weak}");
        let below = Bus { rpm: 2000.0, ..red };
        assert_eq!(frame(&mut h, &below, 0.016).rumble_right, 0);
    }

    #[test]
    fn the_governor_is_learnt_where_the_bus_does_not_say_it() {
        let mut h = Haptics::default();
        let mut bus = Bus {
            throttle: 1.0,
            ..driving()
        };
        for n in (800..=2200).step_by(50) {
            bus.rpm = n as f32;
            frame(&mut h, &bus, 0.05);
            assert_eq!(h.governor, 0.0);
        }
        for _ in 0..20 {
            frame(&mut h, &bus, 0.05);
        }
        assert_eq!(h.governor, 2200.0);
        assert!(
            frame(&mut h, &bus, 0.016).rumble_right > 0
                || frame(&mut h, &bus, 0.05).rumble_right > 0
        );
    }

    #[test]
    fn a_door_shutting_is_a_light_knock_that_dies_away() {
        let mut h = Haptics::default();
        frame(
            &mut h,
            &Bus {
                doors_open: true,
                ..driving()
            },
            0.016,
        );
        let shut = frame(&mut h, &driving(), 0.016);
        assert!(shut.rumble_right > 20 && shut.rumble_right < 60, "{shut:?}");
        let later = frame(&mut h, &driving(), 0.2);
        assert_eq!((later.rumble_left, later.rumble_right), (0, 0));
    }

    #[test]
    fn a_crash_knocks_by_its_energy() {
        let mut h = Haptics::default();
        frame(&mut h, &driving(), 0.016);
        let hit = frame(
            &mut h,
            &Bus {
                crashes: 1,
                last_impact: 200_000.0,
                ..driving()
            },
            0.016,
        );
        assert!(hit.rumble_left > 230);
    }

    #[test]
    fn the_brake_is_free_with_the_retarders_notch_and_knocks_with_the_abs() {
        assert_eq!(brake_trigger(&driving()), Trigger::Off);
        let notch = brake_trigger(&Bus {
            retarder: true,
            ..driving()
        });
        let Trigger::Feedback(z) = notch else {
            panic!("{notch:?}")
        };
        assert_eq!(&z[..3], &[0, 0, 0]);
        assert!(z[3] > z[4]);
        // the script says when its ABS works
        let braking = Bus {
            brake: 0.95,
            kmh: 50.0,
            ..driving()
        };
        assert_eq!(
            brake_trigger(&Bus {
                abs: Some(false),
                ..braking
            }),
            Trigger::Off
        );
        let working = Bus {
            abs: Some(true),
            brake: 0.5,
            ..braking
        };
        assert!(matches!(brake_trigger(&working), Trigger::Vibration { .. }));
        // else braking hard at speed, or sliding
        assert!(matches!(brake_trigger(&braking), Trigger::Vibration { .. }));
        let slide = Bus {
            sliding: true,
            brake: 0.4,
            kmh: 40.0,
            ..driving()
        };
        assert!(matches!(brake_trigger(&slide), Trigger::Vibration { .. }));
        assert_eq!(
            brake_trigger(&Bus {
                kmh: 5.0,
                ..braking
            }),
            Trigger::Off
        );
    }

    #[test]
    fn the_throttle_is_free_unless_a_brake_holds_the_bus() {
        let mut h = Haptics::default();
        assert_eq!(frame(&mut h, &driving(), 0.016).trigger_right, Trigger::Off);
        let out = frame(
            &mut h,
            &Bus {
                stop_brake: true,
                ..driving()
            },
            0.016,
        );
        assert_eq!(out.trigger_right, Trigger::resistance(0, 8));
        assert_eq!(out.lightbar, GREEN);
        assert_eq!(
            throttle_trigger(&Bus {
                parking_brake: true,
                ..driving()
            }),
            Trigger::resistance(0, 8)
        );
    }

    #[test]
    fn the_triggers_follow_the_players_strength() {
        assert_eq!(scaled(Trigger::resistance(0, 8), 0.0), Trigger::Off);
        assert_eq!(
            scaled(Trigger::resistance(0, 8), 0.5),
            Trigger::resistance(0, 4)
        );
        // (a zone with any resistance keeps some)
        assert_eq!(
            scaled(Trigger::resistance(0, 1), 0.2),
            Trigger::resistance(0, 1)
        );
    }

    #[test]
    fn the_light_bar_says_the_most_pressing_first() {
        let h = Haptics::default();
        let indicating = Bus {
            indicator: 1,
            indicator_lamp: Some(true),
            parking_brake: true,
            ..driving()
        };
        assert_eq!(h.lightbar(&indicating), ORANGE);
        assert_eq!(
            h.lightbar(&Bus {
                indicator_lamp: Some(false),
                ..indicating
            })[0],
            20
        );
        assert_eq!(
            h.lightbar(&Bus {
                parking_brake: true,
                ..driving()
            }),
            RED
        );
        let fault = h.lightbar(&Bus {
            electrical_fault: true,
            indicator: 3,
            ..driving()
        });
        assert_eq!((fault[1], fault[2]), (0, 0));
    }

    #[test]
    fn indicators_without_a_lamp_blink_all_the_same() {
        let mut h = Haptics::default();
        let no_lamp = Bus {
            indicator: 2,
            indicator_lamp: None,
            ..driving()
        };
        let lit = frame(&mut h, &no_lamp, 0.1).lightbar;
        let dark = frame(&mut h, &no_lamp, 0.3).lightbar;
        assert_eq!((lit, dark[0]), (ORANGE, 20));
    }

    #[test]
    fn the_player_lights_show_the_direction() {
        assert_eq!(player_leds(&driving()), 0b00100);
        assert_eq!(
            player_leds(&Bus {
                gear: Some(0.0),
                ..driving()
            }),
            0b01010
        );
        assert_eq!(
            player_leds(&Bus {
                reverse: true,
                ..driving()
            }),
            0b10001
        );
        assert_eq!(
            player_leds(&Bus {
                power: false,
                ..driving()
            }),
            0
        );
    }
}
