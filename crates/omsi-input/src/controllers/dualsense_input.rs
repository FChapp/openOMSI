//! A DualSense's gyroscope, touchpad and battery for the game (`dualsense::input`), read in
//! `poll` from the pad's last report.

use super::*;

/// The touchpad's click, as DirectInput numbers the DualSense's buttons.
#[cfg(windows)]
const TOUCHPAD_BUTTON: usize = 13;

impl Controllers {
    /// The gyroscope's look into `out`, and the touchpad as the mouse (Windows, where the pad
    /// is read over HID; elsewhere nothing yet). Nothing while the window is not the one in
    /// front: the cursor is the system's.
    pub(super) fn dualsense_input(&mut self, out: &mut Analog) {
        #[cfg(windows)]
        {
            use crate::dualsense::input::{Gyro, gyro_look, gyro_scale};
            use crate::dualsense::mouse;
            let pad = self
                .devices
                .dualsense
                .as_ref()
                .and_then(|d| d.pads().into_iter().next());
            let Some(pad) = pad.filter(|p| self.focused && !self.off(&p.name)) else {
                self.touchpad = Default::default();
                if std::mem::take(&mut self.touch_click) {
                    mouse::left_button(false);
                }
                return;
            };
            let prefs = self.ds_input;
            out.gyro = gyro_look(&pad.input, gyro_scale(pad.calibration.as_ref()), prefs.gyro);
            if prefs.touch_mouse {
                // (a finger that holds the gyroscope's clutch moves no cursor)
                let finger = pad.input.touches[0].filter(|_| prefs.gyro != Gyro::Touch);
                let (dx, dy) = self.touchpad.step(finger, prefs.touch_speed);
                mouse::move_by(dx, dy);
                for (name, n, down) in &self.raw_buttons {
                    if *n == TOUCHPAD_BUTTON && *name == pad.name && *down != self.touch_click {
                        mouse::left_button(*down);
                        self.touch_click = *down;
                    }
                }
            } else if std::mem::take(&mut self.touch_click) {
                mouse::left_button(false);
            }
        }
        #[cfg(not(windows))]
        let _ = (out, &self.touchpad, self.touch_click, self.ds_input);
    }

    /// The DualSense's battery as it runs low (percent): once at 20 % and once at 10 %.
    pub fn dualsense_battery_low(&mut self) -> Option<u8> {
        #[cfg(windows)]
        {
            let pad = self.devices.dualsense.as_ref()?.pads().into_iter().next()?;
            self.battery.step(pad.input.battery)
        }
        #[cfg(not(windows))]
        {
            let _ = &self.battery;
            None
        }
    }
}
