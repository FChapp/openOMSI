//! What the DualSense's full report adds to its sticks and buttons, made into the game's
//! terms: the gyroscope as a look round (degrees a second), the touchpad as a mouse (pixels
//! to move the cursor by), and the battery as a warning when it runs low. Pure, as
//! `feedback` is: the game says what the player chose (`Prefs`) and does what comes back.

use super::report::{Battery, FEATURE_CALIBRATION_LEN, Input, Touch};

/// When the gyroscope turns the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gyro {
    #[default]
    Off,
    /// Always, as the pad is turned.
    On,
    /// While a finger rests on the touchpad (a clutch: lift it, turn the pad back, put it
    /// down again); the touchpad then moves no cursor.
    Touch,
}

impl Gyro {
    /// The setting's word (`pad_ds_gyro`): `off`, `on` or `touch`.
    pub fn from_setting(s: &str) -> Gyro {
        match s.trim().to_ascii_lowercase().as_str() {
            "on" => Gyro::On,
            "touch" => Gyro::Touch,
            _ => Gyro::Off,
        }
    }
}

/// What the player chose (the settings' `pad_ds_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub gyro: Gyro,
    /// The touchpad moves the mouse and its click is the left button.
    pub touch_mouse: bool,
    /// The cursor's speed (1: a stroke across the touchpad crosses some 1400 pixels).
    pub touch_speed: f32,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            gyro: Gyro::Off,
            touch_mouse: true,
            touch_speed: 1.0,
        }
    }
}

/// Degrees a second per unit of the gyroscope (pitch, yaw, roll), from the pad's calibration
/// (feature report 0x05, as the Linux driver reads it); without one, the nominal 2000°/s
/// over the 16 bits.
pub fn gyro_scale(calibration: Option<&[u8; FEATURE_CALIBRATION_LEN]>) -> [f32; 3] {
    const NOMINAL: f32 = 2000.0 / 32768.0;
    let Some(c) = calibration else {
        return [NOMINAL; 3];
    };
    let le = |i: usize| f32::from(i16::from_le_bytes([c[i], c[i + 1]]));
    let speed_2x = le(19) + le(21);
    let mut scale = [NOMINAL; 3];
    for (k, s) in scale.iter_mut().enumerate() {
        let bias = le(1 + 2 * k);
        let span = (le(7 + 4 * k) - bias).abs() + (le(9 + 4 * k) - bias).abs();
        // (a pad with no calibration written gives zeros: keep the nominal scale)
        if span > 0.0 && speed_2x > 0.0 {
            *s = speed_2x / span;
        }
    }
    scale
}

/// Turns smaller than this (degrees a second) are the hand's trembling and the sensor's
/// drift, not a look round.
const GYRO_FLOOR: f32 = 1.5;

/// How the view is to turn this moment from the gyroscope (degrees a second, right and up):
/// the pad turned left or right about its upright axis turns the head so, tilted up or down
/// it looks up or down.
pub fn gyro_look(input: &Input, scale: [f32; 3], gyro: Gyro) -> [f32; 2] {
    let on = match gyro {
        Gyro::Off => false,
        Gyro::On => true,
        Gyro::Touch => input.touches.iter().any(Option::is_some),
    };
    if !on {
        return [0.0; 2];
    }
    let rate = |k: usize| {
        let v = f32::from(input.gyro[k]) * scale[k];
        if v.abs() < GYRO_FLOOR { 0.0 } else { v }
    };
    // (the yaw counts anticlockwise seen from above: turning the pad right is negative)
    [-rate(1), rate(0)]
}

/// The touchpad as a mouse: how far a finger slid since the last frame, in pixels (whole,
/// the rest kept for the next frame).
#[derive(Clone, Debug, Default)]
pub struct Touchpad {
    last: Option<Touch>,
    rest: [f32; 2],
}

/// Pixels a unit of the touchpad (1920 across) at speed 1.
const TOUCH_GAIN: f32 = 0.75;

impl Touchpad {
    pub fn step(&mut self, touch: Option<Touch>, speed: f32) -> (i32, i32) {
        let moved = match (self.last, touch) {
            // (the same finger: a new one put down starts where it lands, not a jump)
            (Some(a), Some(b)) if a.id == b.id => [
                f32::from(b.x) - f32::from(a.x),
                f32::from(b.y) - f32::from(a.y),
            ],
            _ => {
                self.rest = [0.0; 2];
                [0.0; 2]
            }
        };
        self.last = touch;
        let k = TOUCH_GAIN * speed.clamp(0.1, 5.0);
        let mut out = [0i32; 2];
        for i in 0..2 {
            let v = moved[i] * k + self.rest[i];
            out[i] = v.trunc() as i32;
            self.rest[i] = v - v.trunc();
        }
        (out[0], out[1])
    }
}

/// The battery's warnings: once at 20 % and once at 10 % as it runs down, again after it was
/// charged.
#[derive(Clone, Debug, Default)]
pub struct BatteryWatch {
    /// The lowest level warned about since it was last charged.
    warned: Option<u8>,
}

impl BatteryWatch {
    /// The level to warn about now (percent), if any.
    pub fn step(&mut self, battery: Battery) -> Option<u8> {
        let Battery::Discharging(level) = battery else {
            self.warned = None;
            return None;
        };
        let at = [10, 20].into_iter().find(|t| level <= *t)?;
        if self.warned.is_some_and(|w| w <= at) {
            return None;
        }
        self.warned = Some(at);
        Some(level)
    }
}

#[cfg(test)]
mod tests {
    use super::super::report::{Battery, Buttons, Input, Touch};
    use super::*;

    fn input(gyro: [i16; 3], touches: [Option<Touch>; 2]) -> Input {
        Input {
            left_stick: [128; 2],
            right_stick: [128; 2],
            l2: 0,
            r2: 0,
            buttons: Buttons::default(),
            gyro,
            accel: [0; 3],
            sensor_time: 0,
            touches,
            battery: Battery::Full,
        }
    }

    #[test]
    fn the_calibration_gives_degrees_a_second() {
        // the bytes of a real pad (firmware 0x0558, over Bluetooth)
        let cal: [u8; 41] = [
            0x05, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xb3, 0x22, 0x4d, 0xdd, 0x84, 0x22, 0x7c,
            0xdd, 0xa2, 0x22, 0x5c, 0xdd, 0x1c, 0x02, 0x1c, 0x02, 0xf4, 0x1f, 0xd5, 0xdf, 0x78,
            0x20, 0x6d, 0xe0, 0xf9, 0x1e, 0xd2, 0xde, 0x09, 0x00, 0x9e, 0xb1, 0xf4, 0x4d,
        ];
        let s = gyro_scale(Some(&cal));
        // 1080 / 17766: some 16.4 units a degree a second
        assert!((s[0] - 0.0608).abs() < 0.001, "{s:?}");
        assert!(s.iter().all(|k| (k - 0.0608).abs() < 0.002), "{s:?}");
        assert_eq!(gyro_scale(Some(&[0; 41])), gyro_scale(None));
    }

    #[test]
    fn the_gyroscope_looks_round_only_when_chosen() {
        let s = [0.0608; 3];
        // turning the pad to the right (yaw negative) and tilting it up
        let turning = input([1645, -1645, 0], [None, None]);
        assert_eq!(gyro_look(&turning, s, Gyro::Off), [0.0, 0.0]);
        let [x, y] = gyro_look(&turning, s, Gyro::On);
        assert!(
            (x - 100.0).abs() < 0.5 && (y - 100.0).abs() < 0.5,
            "{x} {y}"
        );
        assert_eq!(gyro_look(&turning, s, Gyro::Touch), [0.0, 0.0]);
        let touched = input(
            [1645, -1645, 0],
            [
                Some(Touch {
                    id: 1,
                    x: 900,
                    y: 500,
                }),
                None,
            ],
        );
        assert!(gyro_look(&touched, s, Gyro::Touch)[0] > 99.0);
        // (the hand's trembling)
        assert_eq!(
            gyro_look(&input([10, -12, 5], [None, None]), s, Gyro::On),
            [0.0, 0.0]
        );
    }

    #[test]
    fn a_finger_moves_the_cursor_without_jumps() {
        let mut t = Touchpad::default();
        let at = |id, x, y| Some(Touch { id, x, y });
        assert_eq!(t.step(at(1, 100, 100), 1.0), (0, 0));
        assert_eq!(t.step(at(1, 140, 80), 1.0), (30, -15));
        // the small moves add up
        assert_eq!(t.step(at(1, 141, 80), 1.0), (0, 0));
        assert_eq!(t.step(at(1, 142, 80), 1.0), (1, 0));
        // lifted, and put down elsewhere
        assert_eq!(t.step(None, 1.0), (0, 0));
        assert_eq!(t.step(at(2, 1800, 900), 1.0), (0, 0));
        assert_eq!(t.step(at(2, 1810, 900), 2.0), (15, 0));
    }

    #[test]
    fn the_battery_warns_at_twenty_and_ten_percent_once() {
        let mut w = BatteryWatch::default();
        assert_eq!(w.step(Battery::Discharging(55)), None);
        assert_eq!(w.step(Battery::Discharging(15)), Some(15));
        assert_eq!(w.step(Battery::Discharging(15)), None);
        assert_eq!(w.step(Battery::Discharging(5)), Some(5));
        assert_eq!(w.step(Battery::Discharging(5)), None);
        // charged, then running down again
        assert_eq!(w.step(Battery::Charging(65)), None);
        assert_eq!(w.step(Battery::Discharging(15)), Some(15));
    }

    #[test]
    fn the_gyro_setting_is_read() {
        assert_eq!(Gyro::from_setting("touch"), Gyro::Touch);
        assert_eq!(Gyro::from_setting("ON"), Gyro::On);
        assert_eq!(Gyro::from_setting("sideways"), Gyro::Off);
    }
}
