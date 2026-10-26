//! The PlayStation 5 pad (DualSense, DualSense Edge) read and driven over raw HID.
//!
//! Windows lists the pad as a DirectInput game controller, but DirectInput only understands
//! its short Bluetooth report: once anything on the computer (Steam, a pad tool, the system's
//! own GameInput) has read a feature report from it, the pad sends only its full report and
//! DirectInput sees it frozen. So on Windows this module reads the pad itself (`device`) and
//! hands it on as DirectInput did - the same name, the same axes, the same button numbers, so a
//! `gamectrler.cfg` made then still fits - and DirectInput's own entry for it is left out.
//! The same reports carry what DirectInput never had: the touchpad, the motion sensors, the
//! battery, and the way back to the pad (rumble, adaptive triggers, lights).

pub mod report;

#[cfg(windows)]
mod device;
#[cfg(windows)]
pub use device::{DualSenses, Pad};

use report::{Buttons, Input};

/// The pad's name when the system gives none (the one Windows gives it).
pub const NAME: &str = "DualSense Wireless Controller";

/// How many buttons it has, numbered as DirectInput numbers them (the HID descriptor's
/// order): square, cross, circle, triangle, L1, R1, L2, R2, Create, Options, L3, R3, PS,
/// touchpad; and the microphone button, which the descriptor leaves out, as 14.
pub const BUTTONS: usize = 15;

fn button_states(b: &Buttons) -> [bool; BUTTONS] {
    [
        b.square, b.cross, b.circle, b.triangle, b.l1, b.r1, b.l2, b.r2, b.create, b.options, b.l3,
        b.r3, b.ps, b.touchpad, b.mic,
    ]
}

/// The buttons pressed (true) and let go between `before` and `now`, by number; the D-pad
/// as up, right, down, left from `hat_base` (DirectInput's first hat).
pub fn button_changes(before: &Buttons, now: &Buttons, hat_base: usize) -> Vec<(usize, bool)> {
    let hat = |b: &Buttons| [b.up, b.right, b.down, b.left];
    let numbered = button_states(before)
        .into_iter()
        .zip(button_states(now))
        .enumerate();
    let hats = hat(before)
        .into_iter()
        .zip(hat(now))
        .enumerate()
        .map(|(k, d)| (hat_base + k, d));
    numbered
        .chain(hats)
        .filter(|(_, (was, is))| was != is)
        .map(|(n, (_, is))| (n, is))
        .collect()
}

/// The axes in DirectInput's slots and range, as it listed the pad: X and Y the left stick,
/// Z and Rz the right one, Rx and Ry L2 and R2 (-1 at rest); -1..1, down and right positive.
pub fn axes(i: &Input) -> Vec<(usize, f32)> {
    let a = |v: u8| f32::from(v) / 127.5 - 1.0;
    vec![
        (0, a(i.left_stick[0])),
        (1, a(i.left_stick[1])),
        (2, a(i.right_stick[0])),
        (3, a(i.l2)),
        (4, a(i.r2)),
        (5, a(i.right_stick[1])),
    ]
}

#[cfg(test)]
mod tests {
    use super::report::{Battery, Input};
    use super::*;

    fn input() -> Input {
        Input {
            left_stick: [0, 255],
            right_stick: [128, 64],
            l2: 0,
            r2: 255,
            buttons: Buttons::default(),
            gyro: [0; 3],
            accel: [0; 3],
            sensor_time: 0,
            touches: [None, None],
            battery: Battery::Full,
        }
    }

    #[test]
    fn axes_are_where_direct_input_had_them() {
        let a = axes(&input());
        let at = |k: usize| a.iter().find(|(s, _)| *s == k).unwrap().1;
        assert_eq!((at(0), at(1)), (-1.0, 1.0));
        assert!(at(2).abs() < 0.01);
        assert_eq!((at(3), at(4)), (-1.0, 1.0)); // L2 released, R2 pressed fully
        assert!((at(5) + 0.5).abs() < 0.01);
    }

    #[test]
    fn buttons_are_numbered_as_direct_input_numbers_them() {
        let none = Buttons::default();
        let pressed = Buttons {
            cross: true,
            options: true,
            touchpad: true,
            mic: true,
            ..none
        };
        assert_eq!(
            button_changes(&none, &pressed, 128),
            vec![(1, true), (9, true), (13, true), (14, true)]
        );
        assert_eq!(button_changes(&pressed, &pressed, 128), vec![]);
        assert_eq!(
            button_changes(&pressed, &none, 128),
            vec![(1, false), (9, false), (13, false), (14, false)]
        );
    }

    #[test]
    fn the_d_pad_is_the_first_hat() {
        let none = Buttons::default();
        let up_right = Buttons {
            up: true,
            right: true,
            ..none
        };
        let right = Buttons {
            right: true,
            ..none
        };
        assert_eq!(
            button_changes(&none, &up_right, 128),
            vec![(128, true), (129, true)]
        );
        assert_eq!(button_changes(&up_right, &right, 128), vec![(128, false)]);
    }
}
