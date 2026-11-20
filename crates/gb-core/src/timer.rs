//! Game Boy timer and divider hardware.
//!
//! The internal divider is a free-running 16-bit counter.
//! TIMA increments on falling edges of a selected divider bit.

/// Game Boy timer peripheral.
#[derive(Debug, Clone, Default)]
pub struct Timer {
    divider: u16,
    tima: u8,
    tma: u8,
    tac: u8,

    reload_delay: u8,
    reloaded_this_cycle: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn initialize_dmg_post_boot(&mut self) {
        self.divider = 0xAB00;
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xFF04 => (self.divider >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8, // TAC's unused upper bits read 1.

            _ => unreachable!("invalid timer register"),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0xFF04 => self.write_div(),
            0xFF05 => self.write_tima(value),
            0xFF06 => self.write_tma(value),
            0xFF07 => self.write_tac(value),
            _ => unreachable!("invalid timer register"),
        }
    }

    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut interrupt_requested = false;

        for _ in 0..cycles {
            self.reloaded_this_cycle = false;

            // An overflow does not immediately reload TIMA. The reload and
            // interrupt happen four T-cycles after the falling edge that
            // caused overflow.
            if self.reload_delay != 0 {
                self.reload_delay -= 1;

                if self.reload_delay == 0 {
                    self.tima = self.tma;
                    self.reloaded_this_cycle = true;
                    interrupt_requested = true;
                }
            }

            let old_input = self.timer_input();

            self.divider = self.divider.wrapping_add(1);

            let new_input = self.timer_input();

            if old_input && !new_input {
                self.increment_tima();
            }
        }

        interrupt_requested
    }

    fn timer_input(&self) -> bool {
        if self.tac & 0x04 == 0 {
            return false;
        }

        let bit = match self.tac & 0x03 {
            0 => 9,
            1 => 3,
            2 => 5,
            3 => 7,
            _ => unreachable!(),
        };

        self.divider & (1 << bit) != 0
    }

    fn increment_tima(&mut self) {
        // Timer increments are suppressed while an overflow reload is
        // pending or taking place.
        if self.reload_delay != 0 || self.reloaded_this_cycle {
            return;
        }

        let (value, overflowed) = self.tima.overflowing_add(1);
        self.tima = value;

        if overflowed {
            self.reload_delay = 4;
        }
    }

    fn write_div(&mut self) {
        let old_input = self.timer_input();

        self.divider = 0;

        if old_input && !self.timer_input() {
            self.increment_tima();
        }
    }

    fn write_tima(&mut self, value: u8) {
        if self.reloaded_this_cycle {
            return;
        }

        self.tima = value;
        self.reload_delay = 0;
    }

    fn write_tma(&mut self, value: u8) {
        self.tma = value;

        if self.reloaded_this_cycle {
            self.tima = value;
        }
    }

    fn write_tac(&mut self, value: u8) {
        let old_input = self.timer_input();

        self.tac = value & 0x07;

        if old_input && !self.timer_input() {
            self.increment_tima();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divider_increments_every_256_cycles() {
        let mut timer = Timer::new();

        timer.tick(255);
        assert_eq!(timer.read(0xFF04), 0);

        timer.tick(1);
        assert_eq!(timer.read(0xFF04), 1);

        timer.tick(256);
        assert_eq!(timer.read(0xFF04), 2);
    }

    #[test]
    fn writing_div_resets_counter() {
        let mut timer = Timer::new();

        timer.tick(512);
        assert_eq!(timer.read(0xFF04), 2);

        timer.write(0xFF04, 0xFF);
        assert_eq!(timer.read(0xFF04), 0);
    }

    #[test]
    fn disabled_timer_does_not_increment_tima() {
        let mut timer = Timer::new();

        timer.tick(4096);

        assert_eq!(timer.read(0xFF05), 0);
        assert_eq!(timer.read(0xFF04), 16);
    }

    #[test]
    fn timer_frequencies() {
        let frequencies = [(0, 1024), (1, 16), (2, 64), (3, 256)];

        for (frequency, period) in frequencies {
            let mut timer = Timer::new();
            timer.write(0xFF07, 0x04 | frequency);

            timer.tick(period - 1);
            assert_eq!(timer.read(0xFF05), 0);

            timer.tick(1);
            assert_eq!(timer.read(0xFF05), 1);

            timer.tick(period);
            assert_eq!(timer.read(0xFF05), 2);
        }
    }

    #[test]
    fn div_reset_can_increment_tima() {
        let mut timer = Timer::new();

        // Enable timer, selecting divider bit 3.
        timer.write(0xFF07, 0x05);

        // Bit 3 becomes high after eight T-cycles.
        timer.tick(8);
        assert_eq!(timer.read(0xFF05), 0);

        // Resetting DIV makes bit 3 fall.
        timer.write(0xFF04, 0);

        assert_eq!(timer.read(0xFF05), 1);
    }

    #[test]
    fn changing_tac_can_increment_tima() {
        let mut timer = Timer::new();

        timer.write(0xFF07, 0x05);
        timer.tick(8);

        // Switch from divider bit 3 (high)
        // to divider bit 9 (low).
        timer.write(0xFF07, 0x04);

        assert_eq!(timer.read(0xFF05), 1);
    }

    #[test]
    fn overflow_reloads_after_four_cycles() {
        let mut timer = Timer::new();

        timer.write(0xFF06, 0x42);
        timer.write(0xFF05, 0xFF);
        timer.write(0xFF07, 0x05);

        // First falling edge causes overflow.
        assert!(!timer.tick(16));
        assert_eq!(timer.read(0xFF05), 0);

        // TIMA remains zero during the delay.
        assert!(!timer.tick(3));
        assert_eq!(timer.read(0xFF05), 0);

        // Reload and interrupt request.
        assert!(timer.tick(1));
        assert_eq!(timer.read(0xFF05), 0x42);
    }

    #[test]
    fn writing_tima_cancels_pending_reload() {
        let mut timer = Timer::new();

        timer.write(0xFF06, 0x42);
        timer.write(0xFF05, 0xFF);
        timer.write(0xFF07, 0x05);

        timer.tick(16);
        timer.tick(2);

        timer.write(0xFF05, 0x99);

        assert!(!timer.tick(2));
        assert_eq!(timer.read(0xFF05), 0x99);
    }

    #[test]
    fn writes_during_reload_follow_hardware_rules() {
        let mut timer = Timer::new();

        timer.write(0xFF05, 0xFF);
        timer.write(0xFF06, 0x42);
        timer.write(0xFF07, 0x05);

        timer.tick(16);
        assert!(timer.tick(4));

        // TIMA writes are ignored on the reload cycle.
        timer.write(0xFF05, 0x99);
        assert_eq!(timer.read(0xFF05), 0x42);

        // TMA writes also update TIMA on that cycle.
        timer.write(0xFF06, 0x77);
        assert_eq!(timer.read(0xFF05), 0x77);
    }

    #[test]
    fn tac_unused_bits_read_as_one() {
        let mut timer = Timer::new();

        timer.write(0xFF07, 0xFF);

        assert_eq!(timer.read(0xFF07), 0xFF);
    }

    #[test]
    fn divider_wraps() {
        let mut timer = Timer::new();

        timer.tick(65536);

        assert_eq!(timer.read(0xFF04), 0);
    }
}
