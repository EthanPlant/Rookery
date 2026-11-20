use crate::{Bus, FLAG_C, FLAG_H, FLAG_N, FLAG_Z, Instruction, Opcode, Registers, Sm83};

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
        (step.pc, step.opcode, step.instruction, step.cycles),
        (0x0100, Opcode::Base(0), Instruction::Nop, 4)
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
    assert_eq!(result.opcode, Opcode::Cb(0x38));
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
