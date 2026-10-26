//! What the bus says through a DualSense, frame by frame: its rumble, its adaptive triggers
//! and its lights. Pure: the game hands in what the bus does (`Bus`) and the shaking the
//! force feedback already works out (`Shaking`), and gets the pad's whole `Output` back.
//!
//! - rumble: the engine's buzz and the script's shaking (`FF_Vib_Amp`, in its pulses) on both
//!   motors, the heavy one stronger; the road only where it is felt - a pothole, a kerb - and
//!   not its grain, which on a pad is a buzz that never stops; and small knocks of their own:
//!   a door shutting, a gear going in, the indicator's relay, a crash;
//! - L2, the brake: free; on a bus with a retarder free over the retarder's travel and then a
//!   notch where the brakes begin; it knocks like ABS when braked hard at speed, or while the
//!   wheels slide;
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
    /// The brake pedal, 0..1, and whether the tyres slide (the body no longer holds the
    /// road), with the speed.
    pub brake: f32,
    pub sliding: bool,
    pub kmh: f32,
    /// Crashes so far and the energy of the latest (J).
    pub crashes: u32,
    pub last_impact: f32,
}

/// The bus's shaking as the force feedback works it out, before the pad's strength.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shaking {
    /// The script's (`FF_Vib_Amp`, 0..1) and its pulses (`FF_Vib_Period`, hundredths of a
    /// second, on for half of it; 0 steady).
    pub script: f32,
    pub period: f32,
    /// A wheel's jolt (a pothole, a kerb: 0..1, held and dying away).
    pub bump: f32,
    /// The engine's buzz (its trembling as a wheel gets it, -1..1).
    pub engine: f32,
}

/// Jolts smaller than this are the road's own unevenness, not felt on the pad.
const BUMP_FLOOR: f32 = 0.15;

/// Warm white, the light bar at rest.
pub const WARM_WHITE: [u8; 3] = [255, 170, 90];
const ORANGE: [u8; 3] = [255, 80, 0];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 230, 40];

/// The pad's state over the frames: the knocks dying away, the engine's buzz eased, and the
/// last frame's bus to tell what changed.
#[derive(Clone, Debug, Default)]
pub struct Haptics {
    t: f32,
    /// The knocks on the heavy (left) and the light (right) motor, 0..1, dying away.
    knock: [f32; 2],
    /// The engine's buzz, eased (a rumble motor cannot follow its waves).
    engine: f32,
    last: Option<Bus>,
}

/// How fast a knock dies away (its strength a second).
const KNOCK_FADE: f32 = 8.0;

impl Haptics {
    /// The pad's output this frame. `vibration`: the player's strength for the pad (0..2);
    /// `triggers`: for the triggers (0 off .. 1).
    pub fn frame(
        &mut self,
        bus: &Bus,
        shaking: Shaking,
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
        }
        self.last = Some(*bus);
        if !bus.driving {
            self.knock = [0.0; 2];
            self.engine = 0.0;
            return Output {
                lightbar: WARM_WHITE,
                ..Output::default()
            };
        }

        let ms = shaking.period * 10.0;
        let pulse_on = ms < 20.0 || (self.t * 1000.0) % ms < ms / 2.0;
        self.engine += (shaking.engine.abs().min(1.0) - self.engine) * (dt / 0.3).min(1.0);
        let script = if pulse_on {
            shaking.script.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bump = ((shaking.bump - BUMP_FLOOR) / (1.0 - BUMP_FLOOR)).clamp(0.0, 1.0);
        let shake = script.max(self.engine);
        let motor = |v: f32| ((v * vibration.clamp(0.0, 2.0)).clamp(0.0, 1.0) * 255.0) as u8;
        Output {
            rumble_left: motor(shake + bump + self.knock[0]),
            rumble_right: motor(shake * 0.5 + bump * 0.5 + self.knock[1]),
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

/// L2: ABS's knocking when braked hard at speed or while the wheels slide; else free, with
/// the notch where the retarder's travel ends on a bus that has one.
fn brake_trigger(bus: &Bus) -> Trigger {
    if bus.kmh > 10.0 && (bus.brake > 0.85 || (bus.sliding && bus.brake > 0.2)) {
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

    fn shaking(script: f32, period: f32) -> Shaking {
        Shaking {
            script,
            period,
            ..Shaking::default()
        }
    }

    fn driving() -> Bus {
        Bus {
            driving: true,
            power: true,
            gear: Some(1.0),
            ..Bus::default()
        }
    }

    fn still() -> Shaking {
        Shaking::default()
    }

    #[test]
    fn at_rest_the_pad_is_warm_white_and_still() {
        let mut h = Haptics::default();
        let out = h.frame(&Bus::default(), shaking(1.0, 0.0), 1.0, 1.0, 0.016);
        assert_eq!(out.lightbar, WARM_WHITE);
        assert_eq!((out.rumble_left, out.rumble_right), (0, 0));
        assert_eq!(
            (out.trigger_left, out.trigger_right),
            (Trigger::Off, Trigger::Off)
        );
    }

    #[test]
    fn the_shaking_is_scaled_and_pulsed() {
        let mut h = Haptics::default();
        let out = h.frame(&driving(), shaking(0.5, 0.0), 1.0, 1.0, 0.016);
        assert_eq!((out.rumble_left, out.rumble_right), (127, 63));
        let out = h.frame(&driving(), shaking(0.5, 0.0), 0.0, 1.0, 0.016);
        assert_eq!(out.rumble_left, 0);
        // pulses of 40 hundredths: on for the first 0.2 s, off for the next
        let mut h = Haptics::default();
        let on = h.frame(&driving(), shaking(1.0, 40.0), 1.0, 1.0, 0.1);
        let off = h.frame(&driving(), shaking(1.0, 40.0), 1.0, 1.0, 0.2);
        assert_eq!((on.rumble_left, off.rumble_left), (255, 0));
    }

    #[test]
    fn only_real_jolts_of_the_road_are_felt() {
        let mut h = Haptics::default();
        let rough = Shaking {
            bump: 0.1,
            ..still()
        };
        assert_eq!(h.frame(&driving(), rough, 1.0, 1.0, 0.016).rumble_left, 0);
        let kerb = Shaking {
            bump: 0.8,
            ..still()
        };
        assert!(h.frame(&driving(), kerb, 1.0, 1.0, 0.016).rumble_left > 150);
    }

    #[test]
    fn the_engine_buzzes_eased() {
        let mut h = Haptics::default();
        let engine = Shaking {
            engine: -0.4,
            ..still()
        };
        let first = h.frame(&driving(), engine, 1.0, 1.0, 0.016).rumble_left;
        for _ in 0..120 {
            h.frame(&driving(), engine, 1.0, 1.0, 0.016);
        }
        let settled = h.frame(&driving(), engine, 1.0, 1.0, 0.016).rumble_left;
        assert!(
            first < settled && settled > 95 && settled <= 102,
            "{first} {settled}"
        );
    }

    #[test]
    fn a_door_shutting_is_a_light_knock_that_dies_away() {
        let mut h = Haptics::default();
        h.frame(
            &Bus {
                doors_open: true,
                ..driving()
            },
            still(),
            1.0,
            1.0,
            0.016,
        );
        let shut = h.frame(&driving(), still(), 1.0, 1.0, 0.016);
        assert!(shut.rumble_right > 20 && shut.rumble_right < 60, "{shut:?}");
        let later = h.frame(&driving(), still(), 1.0, 1.0, 0.2);
        assert_eq!((later.rumble_left, later.rumble_right), (0, 0));
    }

    #[test]
    fn a_crash_knocks_by_its_energy() {
        let mut h = Haptics::default();
        h.frame(&driving(), still(), 1.0, 1.0, 0.016);
        let hit = h.frame(
            &Bus {
                crashes: 1,
                last_impact: 200_000.0,
                ..driving()
            },
            still(),
            1.0,
            1.0,
            0.016,
        );
        assert!(hit.rumble_left > 230);
    }

    #[test]
    fn the_brake_is_free_with_the_retarders_notch_and_knocks_when_braked_hard() {
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
        let hard = Bus {
            brake: 0.95,
            kmh: 50.0,
            ..driving()
        };
        assert!(matches!(brake_trigger(&hard), Trigger::Vibration { .. }));
        let slide = Bus {
            sliding: true,
            brake: 0.4,
            kmh: 40.0,
            ..driving()
        };
        assert!(matches!(brake_trigger(&slide), Trigger::Vibration { .. }));
        // (at a crawl, braking hard is only stopping)
        assert_eq!(brake_trigger(&Bus { kmh: 5.0, ..hard }), Trigger::Off);
    }

    #[test]
    fn the_throttle_is_free_unless_a_brake_holds_the_bus() {
        let mut h = Haptics::default();
        assert_eq!(
            h.frame(&driving(), still(), 1.0, 1.0, 0.016).trigger_right,
            Trigger::Off
        );
        let out = h.frame(
            &Bus {
                stop_brake: true,
                ..driving()
            },
            still(),
            1.0,
            1.0,
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
        let lit = h.frame(&no_lamp, still(), 1.0, 1.0, 0.1).lightbar;
        let dark = h.frame(&no_lamp, still(), 1.0, 1.0, 0.3).lightbar;
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
