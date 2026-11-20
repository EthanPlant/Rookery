use std::assert_matches;

use crate::{Bus, FLAG_C, FLAG_H, FLAG_N, FLAG_Z, Instruction, Opcode, Registers, Sm83, StepKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read(u16),
    Write(u16, u8),
}

struct TestBus {
    memory: Box<[u8; 0x10000]>,
    accesses: Vec<Access>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            memory: Box::new([0; 0x10000]),
            accesses: Vec::new(),
        }
    }

    fn load(&mut self, address: u16, program: &[u8]) {
        let start = usize::from(address);
        self.memory[start..start + program.len()].copy_from_slice(program);
    }
}

impl Bus for TestBus {
    fn read8(&mut self, address: u16) -> u8 {
        self.accesses.push(Access::Read(address));
        self.memory[usize::from(address)]
    }

    fn write8(&mut self, address: u16, value: u8) {
        self.accesses.push(Access::Write(address, value));
        self.memory[usize::from(address)] = value;
    }

    fn pending_interrupts(&self) -> u8 {
        self.memory[0xFFFF] & self.memory[0xFF0F] & 0x1F
    }

    fn acknowledge_interrupt(&mut self, mask: u8) {
        let flags = self.read8(0xFF0F);
        self.write8(0xFF0F, flags & !mask);
    }
}

fn cpu_at(pc: u16) -> Sm83 {
    Sm83::with_registers(Registers {
        pc,
        ..Registers::default()
    })
}

#[test]
fn register_pairs_and_flag_mask() {
    let mut regs = Registers::default();
    regs.set_af(0x12FF);
    regs.set_bc(0x3456);
    regs.set_de(0x789A);
    regs.set_hl(0xBCDE);
    assert_eq!(
        (regs.af(), regs.bc(), regs.de(), regs.hl()),
        (0x12F0, 0x3456, 0x789A, 0xBCDE)
    );
    assert_eq!(regs.f, FLAG_Z | FLAG_N | FLAG_H | FLAG_C);
}

#[test]
fn nop_and_trace_do_not_double_fetch() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x00]);
    let mut cpu = cpu_at(0x0100);
    let step = cpu.step_with_trace(&mut bus).unwrap();
    assert_eq!(
        (step.pc, step.kind, step.cycles),
        (
            0x0100,
            StepKind::Instruction {
                opcode: Opcode::Base(0),
                instruction: Instruction::Nop
            },
            4
        )
    );
    assert_eq!(cpu.registers().pc, 0x0101);
    assert_eq!(bus.accesses, vec![Access::Read(0x0100)]);
}

#[test]
fn ld_register_family_decodes_from_bit_fields() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x41]); // LD B,C
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        c: 0x42,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert_eq!(cpu.registers().b, 0x42);
    assert_eq!(bus.accesses, vec![Access::Read(0x0100)]);
}

#[test]
fn ld_from_hl_reads_memory_and_costs_eight_t_cycles() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x46]); // LD B,(HL)
    bus.memory[0xC123] = 0x42;
    let mut regs = Registers {
        pc: 0x0100,
        ..Registers::default()
    };
    regs.set_hl(0xC123);
    let mut cpu = Sm83::with_registers(regs);
    assert_eq!(cpu.step(&mut bus).unwrap(), 8);
    assert_eq!(cpu.registers().b, 0x42);
    assert_eq!(
        bus.accesses,
        vec![Access::Read(0x0100), Access::Read(0xC123)]
    );
}

#[test]
fn ld_indirect_hl_immediate_takes_twelve_t_cycles() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x36, 0x9A]);
    let mut regs = Registers {
        pc: 0x0100,
        ..Registers::default()
    };
    regs.set_hl(0xC123);
    let mut cpu = Sm83::with_registers(regs);
    assert_eq!(cpu.step(&mut bus).unwrap(), 12);
    assert_eq!(bus.memory[0xC123], 0x9A);
}

#[test]
fn sub_uses_operand_not_constant_one() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x90]); // SUB B
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        a: 0x40,
        b: 0x12,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert_eq!(cpu.registers().a, 0x2E);
    assert_eq!(cpu.registers().f, FLAG_N | FLAG_H);
}

#[test]
fn and_immediate_takes_eight_t_cycles() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0xE6, 0x00]);
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        a: 0xA5,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 8);
    assert_eq!(cpu.registers().a, 0);
    assert_eq!(cpu.registers().f, FLAG_Z | FLAG_H);
}

#[test]
fn cb_srl_actually_shifts_and_sets_carry() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0xCB, 0x38]); // SRL B
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        b: 0x03,
        ..Registers::default()
    });
    let result = cpu.step_with_trace(&mut bus).unwrap();
    assert_matches!(
        result.kind,
        StepKind::Instruction {
            opcode: Opcode::Cb(0x38),
            ..
        }
    );
    assert_eq!(result.cycles, 8);
    assert_eq!(cpu.registers().b, 0x01);
    assert_eq!(cpu.registers().f, FLAG_C);
}

#[test]
fn cb_bit_hl_preserves_carry_and_has_memory_timing() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0xCB, 0x46]); // BIT 0,(HL)
    bus.memory[0xC000] = 0;
    let mut regs = Registers {
        pc: 0x0100,
        f: FLAG_C,
        ..Registers::default()
    };
    regs.set_hl(0xC000);
    let mut cpu = Sm83::with_registers(regs);
    assert_eq!(cpu.step(&mut bus).unwrap(), 12);
    assert_eq!(cpu.registers().f, FLAG_Z | FLAG_H | FLAG_C);
    assert_eq!(bus.accesses.last(), Some(&Access::Read(0xC000)));
}

#[test]
fn rra_clears_z_even_with_a_zero_result() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x1F]); // RRA
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        a: 1,
        f: FLAG_Z,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert_eq!(cpu.registers().a, 0);
    assert_eq!(cpu.registers().f, FLAG_C);
}

#[test]
fn add_hl_preserves_z() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x09]); // ADD HL,BC
    let mut regs = Registers {
        pc: 0x0100,
        f: FLAG_Z,
        ..Registers::default()
    };
    regs.set_hl(0x0FFF);
    regs.set_bc(1);
    let mut cpu = Sm83::with_registers(regs);
    assert_eq!(cpu.step(&mut bus).unwrap(), 8);
    assert_eq!(cpu.registers().hl(), 0x1000);
    assert_eq!(cpu.registers().f, FLAG_Z | FLAG_H);
}

#[test]
fn call_pushes_high_then_low_and_uses_24_t_cycles() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0xCD, 0x34, 0x12]);
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        sp: 0xFFFE,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 24);
    assert_eq!(cpu.registers().pc, 0x1234);
    assert_eq!(cpu.registers().sp, 0xFFFC);
    assert_eq!(
        &bus.accesses[3..],
        &[Access::Write(0xFFFD, 0x01), Access::Write(0xFFFC, 0x03),]
    );
}

#[test]
fn ld_absolute_a_has_correct_cycle_count() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0xEA, 0x34, 0xC1]);
    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        a: 0xAB,
        ..Registers::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 16);
    assert_eq!(bus.memory[0xC134], 0xAB);
}

#[test]
fn daa_corrects_low_nibble_after_addition() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x27]);
    let mut cpu = Sm83::with_registers(Registers {
        a: 0x0A,
        f: 0x00,
        pc: 0x0100,
        ..Default::default()
    });
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert_eq!(cpu.registers.a, 0x10);
    assert_eq!(cpu.registers.f, 0x00);
}

#[test]
fn daa_sets_zero_and_carry_on_decimal_overflow() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x27]);
    let mut cpu = Sm83::with_registers(Registers {
        a: 0x9A,
        f: 0x00,
        pc: 0x0100,
        ..Default::default()
    });
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.registers.a, 0x00);
    assert_eq!(cpu.registers.f, FLAG_Z | FLAG_C);
}

#[test]
fn daa_corrects_half_borrow_after_subtraction() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x27]);
    let mut cpu = Sm83::with_registers(Registers {
        a: 0x0F,
        f: FLAG_H | FLAG_N,
        pc: 0x0100,
        ..Default::default()
    });
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.registers.a, 0x09);
    assert_eq!(cpu.registers.f, FLAG_N);
}

#[test]
fn daa_corrects_half_borrow_and_half_carry_after_subtraction() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x27]);
    let mut cpu = Sm83::with_registers(Registers {
        a: 0xFF,
        f: FLAG_N | FLAG_H | FLAG_C,
        pc: 0x0100,
        ..Default::default()
    });
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.registers.a, 0x99);
    assert_eq!(cpu.registers.f, FLAG_N | FLAG_C);
}

#[test]
fn daa_sets_zero_flag_for_zero_result() {
    let mut bus = TestBus::new();
    bus.load(0x0100, &[0x27]);
    let mut cpu = Sm83::with_registers(Registers {
        a: 0x00,
        f: 0x00,
        pc: 0x0100,
        ..Default::default()
    });
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.registers.a, 0x00);
    assert_eq!(cpu.registers.f, FLAG_Z);
}

#[test]
fn ei_enables_interrupts_after_next_instruction() {
    let mut bus = TestBus::new();

    // EI; NOP; NOP
    bus.load(0x0100, &[0xFB, 0x00, 0x00]);

    // Timer interrupt already requested and enabled.
    bus.memory[0xFFFF] = 0x04;
    bus.memory[0xFF0F] = 0x04;

    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        sp: 0xFFFE,
        ..Default::default()
    });

    // EI executes, but IME remains disabled.
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert!(!cpu.status().ime);
    assert_eq!(cpu.registers().pc, 0x0101);

    // The following NOP must execute before IRQ.
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert_eq!(cpu.registers().pc, 0x0102);
    assert!(cpu.status().ime);

    // Now the interrupt is serviced.
    let result = cpu.step_with_trace(&mut bus).unwrap();

    assert_eq!(result.cycles, 20);
    assert_eq!(result.kind, StepKind::Interrupt { vector: 0x0050 });

    assert_eq!(cpu.registers().pc, 0x0050);
    assert_eq!(cpu.registers().sp, 0xFFFC);

    // Return address is 0x0102.
    assert_eq!(bus.memory[0xFFFC], 0x02);
    assert_eq!(bus.memory[0xFFFD], 0x01);

    assert!(!cpu.status().ime);
    assert_eq!(bus.memory[0xFF0F] & 0x04, 0);
}

#[test]
fn di_cancels_pending_ei() {
    let mut bus = TestBus::new();

    // EI; DI; NOP
    bus.load(0x0100, &[0xFB, 0xF3, 0x00]);

    bus.memory[0xFFFF] = 0x04;
    bus.memory[0xFF0F] = 0x04;

    let mut cpu = cpu_at(0x0100);

    cpu.step(&mut bus).unwrap();
    assert!(!cpu.status().ime);

    cpu.step(&mut bus).unwrap();
    assert!(!cpu.status().ime);
    assert_eq!(cpu.status().ime_enable_delay, 0);

    // The interrupt must not run.
    cpu.step(&mut bus).unwrap();

    assert_eq!(cpu.registers().pc, 0x0103);
    assert_eq!(bus.memory[0xFF0F] & 0x04, 0x04);
}

#[test]
fn interrupts_are_serviced_in_priority_order() {
    let mut bus = TestBus::new();

    // All interrupt types enabled.
    bus.memory[0xFFFF] = 0x1F;

    // VBlank, Timer, and Joypad requested.
    bus.memory[0xFF0F] = 0b0001_0101;

    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x1234,
        sp: 0xFFFE,
        ..Default::default()
    });

    cpu.control.ime = true;

    let result = cpu.step_with_trace(&mut bus).unwrap();

    // VBlank wins over Timer and Joypad.
    assert_eq!(result.kind, StepKind::Interrupt { vector: 0x0040 });

    assert_eq!(cpu.registers().pc, 0x0040);

    // Only the VBlank request was acknowledged.
    assert_eq!(bus.memory[0xFF0F], 0b0001_0100);
}

#[test]
fn ie_masks_interrupt_requests() {
    let mut bus = TestBus::new();

    // Only the Timer interrupt is enabled.
    bus.memory[0xFFFF] = 0x04;

    // All five interrupts are requested.
    bus.memory[0xFF0F] = 0x1F;

    let mut cpu = cpu_at(0x0100);
    cpu.control.ime = true;

    let result = cpu.step_with_trace(&mut bus).unwrap();

    assert_eq!(result.kind, StepKind::Interrupt { vector: 0x0050 });

    assert_eq!(bus.memory[0xFF0F] & 0x1F, 0x1B);
}

#[test]
fn reti_restores_pc_and_immediately_enables_ime() {
    let mut bus = TestBus::new();

    // RETI at the timer interrupt vector.
    bus.load(0x0050, &[0xD9]);

    // Return address on the stack: 0x1234.
    bus.memory[0xFFFC] = 0x34;
    bus.memory[0xFFFD] = 0x12;

    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0050,
        sp: 0xFFFC,
        ..Default::default()
    });

    assert_eq!(cpu.step(&mut bus).unwrap(), 16);

    assert_eq!(cpu.registers().pc, 0x1234);
    assert_eq!(cpu.registers().sp, 0xFFFE);
    assert!(cpu.status().ime);
}

#[test]
fn halt_idles_until_an_interrupt_is_pending() {
    let mut bus = TestBus::new();

    // HALT; NOP
    bus.load(0x0100, &[0x76, 0x00]);

    bus.memory[0xFFFF] = 0x04;

    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        sp: 0xFFFE,
        ..Default::default()
    });

    cpu.control.ime = true;

    // Execute HALT.
    assert_eq!(cpu.step(&mut bus).unwrap(), 4);
    assert!(cpu.status().halted);
    assert_eq!(cpu.registers().pc, 0x0101);

    // No interrupt yet: idle for four cycles.
    let idle = cpu.step_with_trace(&mut bus).unwrap();

    assert_eq!(idle.kind, StepKind::Halted);
    assert_eq!(idle.cycles, 4);
    assert_eq!(cpu.registers().pc, 0x0101);

    // Request a timer interrupt.
    bus.memory[0xFF0F] = 0x04;

    let result = cpu.step_with_trace(&mut bus).unwrap();

    assert_eq!(result.kind, StepKind::Interrupt { vector: 0x0050 });

    assert!(!cpu.status().halted);
    assert_eq!(cpu.registers().pc, 0x0050);
}

#[test]
fn halt_wakes_without_servicing_when_ime_is_disabled() {
    let mut bus = TestBus::new();

    // HALT; NOP
    bus.load(0x0100, &[0x76, 0x00]);

    bus.memory[0xFFFF] = 0x04;

    let mut cpu = cpu_at(0x0100);

    cpu.step(&mut bus).unwrap();

    assert!(cpu.status().halted);

    // Still no interrupt request.
    assert_eq!(
        cpu.step_with_trace(&mut bus).unwrap().kind,
        StepKind::Halted
    );

    // Request an enabled interrupt.
    bus.memory[0xFF0F] = 0x04;

    // IME is disabled, so resume at the NOP.
    cpu.step(&mut bus).unwrap();

    assert!(!cpu.status().halted);
    assert_eq!(cpu.registers().pc, 0x0102);

    // The interrupt request remains pending.
    assert_eq!(bus.memory[0xFF0F] & 0x04, 0x04);
}

#[test]
fn halt_bug_suppresses_next_opcode_pc_increment() {
    let mut bus = TestBus::new();

    // HALT
    // LD A, $42
    bus.load(0x0100, &[0x76, 0x3E, 0x42]);

    // Interrupt pending before HALT, with IME disabled.
    bus.memory[0xFFFF] = 0x04;
    bus.memory[0xFF0F] = 0x04;

    let mut cpu = cpu_at(0x0100);

    cpu.step(&mut bus).unwrap();

    assert!(!cpu.status().halted);
    assert_eq!(cpu.registers().pc, 0x0101);

    // The opcode 0x3E is fetched without incrementing PC.
    // The immediate byte therefore also reads 0x3E.
    cpu.step(&mut bus).unwrap();

    assert_eq!(cpu.registers().a, 0x3E);
    assert_eq!(cpu.registers().pc, 0x0102);
}

#[test]
fn ei_halt_with_pending_irq_returns_to_halt() {
    let mut bus = TestBus::new();

    // EI; HALT; NOP
    bus.load(0x0100, &[0xFB, 0x76, 0x00]);

    bus.memory[0xFFFF] = 0x04;
    bus.memory[0xFF0F] = 0x04;

    let mut cpu = Sm83::with_registers(Registers {
        pc: 0x0100,
        sp: 0xFFFE,
        ..Default::default()
    });

    cpu.step(&mut bus).unwrap(); // EI
    cpu.step(&mut bus).unwrap(); // HALT bug

    assert!(cpu.status().ime);

    let result = cpu.step_with_trace(&mut bus).unwrap();

    assert_eq!(result.kind, StepKind::Interrupt { vector: 0x0050 });

    // Return to HALT at 0x0101, not NOP at 0x0102.
    assert_eq!(bus.memory[0xFFFC], 0x01);
    assert_eq!(bus.memory[0xFFFD], 0x01);
}
