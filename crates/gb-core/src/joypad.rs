use rookery_common::Buttons;

#[derive(Debug, Clone)]
pub struct Joypad {
    select: u8,

    buttons: Buttons,
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            select: 0x30,
            buttons: Buttons::default(),
        }
    }

    pub fn post_boot() -> Self {
        Self {
            select: 0x00,
            buttons: Buttons::default(),
        }
    }

    pub fn read(&self) -> u8 {
        let pressed = self.buttons.bits() as u8;

        let mut lines = 0x0F;

        if self.select & 0x10 == 0 {
            lines &= !(pressed & 0x0F);
        }

        if self.select & 0x20 == 0 {
            lines &= !(pressed >> 4)
        }

        0xC0 | self.select | (lines & 0x0F)
    }

    pub fn write(&mut self, value: u8) -> bool {
        let previous = self.read();

        self.select = value & 0x30;

        self.has_falling_edge(previous)
    }

    pub fn set_buttons(&mut self, buttons: Buttons) -> bool {
        let previous = self.read();

        self.buttons = buttons;

        self.has_falling_edge(previous)
    }

    pub fn buttons(&self) -> Buttons {
        self.buttons
    }

    fn has_falling_edge(&self, previous: u8) -> bool {
        let current = self.read();

        (previous & !current & 0x0F) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rookery_common::Button;

    #[test]
    fn initial_register_values() {
        assert_eq!(Joypad::new().read(), 0xFF);
        assert_eq!(Joypad::post_boot().read(), 0xCF);
    }

    #[test]
    fn selects_direction_buttons() {
        let mut joypad = Joypad::new();

        let mut buttons = Buttons::new();
        buttons.set(Button::Right, true);
        buttons.set(Button::Up, true);

        joypad.set_buttons(buttons);

        // Select the D-pad.
        joypad.write(0x20);

        // Bits 0 and 2 are low.
        assert_eq!(joypad.read(), 0xEA);
    }

    #[test]
    fn selects_action_buttons() {
        let mut joypad = Joypad::new();

        let mut buttons = Buttons::new();
        buttons.set(Button::A, true);
        buttons.set(Button::Start, true);

        joypad.set_buttons(buttons);

        // Select action buttons.
        joypad.write(0x10);

        // Bits 0 and 3 are low.
        assert_eq!(joypad.read(), 0xD6);
    }

    #[test]
    fn both_groups_combine_their_signals() {
        let mut joypad = Joypad::new();

        let mut buttons = Buttons::new();
        buttons.set(Button::Right, true);
        buttons.set(Button::B, true);

        joypad.set_buttons(buttons);

        // Select both groups.
        joypad.write(0x00);

        // Right clears bit 0, B clears bit 1.
        assert_eq!(joypad.read(), 0xCC);
    }

    #[test]
    fn no_selection_hides_pressed_buttons() {
        let mut joypad = Joypad::new();

        let mut buttons = Buttons::new();
        buttons.set(Button::A, true);
        buttons.set(Button::Down, true);

        assert!(!joypad.set_buttons(buttons));

        joypad.write(0x30);

        assert_eq!(joypad.read(), 0xFF);
    }

    #[test]
    fn only_selection_bits_are_writable() {
        let mut joypad = Joypad::new();

        joypad.write(0x00);

        assert_eq!(joypad.read(), 0xCF);
    }

    #[test]
    fn pressing_selected_button_requests_interrupt() {
        let mut joypad = Joypad::new();
        joypad.write(0x20);

        let mut buttons = Buttons::new();
        buttons.set(Button::Right, true);

        assert!(joypad.set_buttons(buttons));

        // Keeping the same button held is not a new edge.
        assert!(!joypad.set_buttons(buttons));

        // Releasing a button produces a rising edge.
        buttons.set(Button::Right, false);
        assert!(!joypad.set_buttons(buttons));

        // Pressing it again produces another falling edge.
        buttons.set(Button::Right, true);
        assert!(joypad.set_buttons(buttons));
    }

    #[test]
    fn selection_changes_can_request_interrupt() {
        let mut joypad = Joypad::new();

        let mut buttons = Buttons::new();
        buttons.set(Button::A, true);

        // No group selected: no interrupt.
        assert!(!joypad.set_buttons(buttons));

        // Selecting the group containing held A
        // makes input bit 0 fall.
        assert!(joypad.write(0x10));

        // Repeating the selection changes nothing.
        assert!(!joypad.write(0x10));

        // Deselecting the group raises the line.
        assert!(!joypad.write(0x30));

        // Selecting it again creates a new falling edge.
        assert!(joypad.write(0x10));
    }

    #[test]
    fn gba_shoulder_buttons_are_ignored() {
        let mut joypad = Joypad::post_boot();

        let mut buttons = Buttons::new();
        buttons.set(Button::L, true);
        buttons.set(Button::R, true);

        assert!(!joypad.set_buttons(buttons));
        assert_eq!(joypad.read(), 0xCF);
    }
}
