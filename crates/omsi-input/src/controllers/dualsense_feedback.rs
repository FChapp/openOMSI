//! The DualSense's rumble, adaptive triggers and lights from the bus (`dualsense::feedback`),
//! with the shaking the force feedback works out for every rumbling pad (`rumble_feedback`).

use super::*;
use crate::dualsense::feedback::Bus;

impl Controllers {
    /// After `feedback`: what the DualSenses show of `bus` this frame (Windows, where they
    /// are read over HID; elsewhere nothing yet).
    pub fn dualsense_feedback(&mut self, bus: Bus, dt: f32) {
        #[cfg(windows)]
        {
            let Some(ds) = self.devices.dualsense.as_ref() else {
                return;
            };
            let Some(name) = ds.pads().into_iter().next().map(|p| p.name) else {
                return;
            };
            // (as for every pad's rumble: the force feedback switched on, the window focused,
            // the pad not switched off in the settings)
            let on = self.enabled && self.ff_enabled && self.focused && !self.off(&name);
            let bus = Bus {
                driving: bus.driving && on,
                ..bus
            };
            let scale = rumble_scale(&self.cfg, &name);
            let out = self.haptics.frame(&bus, self.pad_rumble, scale, 1.0, dt);
            ds.set_output(out);
        }
        #[cfg(not(windows))]
        let _ = (bus, dt, &self.haptics, self.pad_rumble);
    }
}
