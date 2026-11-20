use crate::{
    Address8, AluOp, Bus, CbOperation, Condition, CpuError, Instruction, Opcode, Operand8,
    Register16, RotateKind, Sm83, Source8, instruction::DecodedInstruction,
};

impl Sm83 {
    pub(crate) fn decode<B: Bus>(&mut self, bus: &mut B) -> Result<DecodedInstruction, CpuError> {
        let pc = self.registers.pc;
        let first = self.fetch_opcode(bus);
        let (opcode, instruction) = if first == 0xCB {
            let second = self.fetch8(bus);
            (Opcode::Cb(second), cb_instruction(second))
        } else {
            (Opcode::Base(first), self.decode_base(bus, pc, first)?)
        };
        Ok(DecodedInstruction {
            pc,
            opcode,
            instruction,
        })
    }

    fn decode_base<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u16,
        code: u8,
    ) -> Result<Instruction, CpuError> {
        let instruction = match code {
            0x00 => Instruction::Nop,

            // 01dddsss, except 0x76 (HALT): all LD r8, r8
            0x40..=0x7F if code != 0x76 => Instruction::Load8 {
                dst: Operand8::from_bits(code >> 3),
                src: Source8::Operand(Operand8::from_bits(code)),
            },

            // 00ddd110: LD r8, d8
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => Instruction::Load8 {
                dst: Operand8::from_bits(code >> 3),
                src: Source8::Immediate(self.fetch8(bus)),
            },

            // 00pp0001: LD r16, d16.
            0x01 | 0x11 | 0x21 | 0x31 => Instruction::Load16 {
                dst: pair(code >> 4),
                value: self.fetch16(bus),
            },

            // Indirect transfers using A.
            0x02 => Instruction::StoreATo(Address8::BC),
            0x12 => Instruction::StoreATo(Address8::DE),
            0x22 => Instruction::StoreATo(Address8::HLInc),
            0x32 => Instruction::StoreATo(Address8::HLDec),
            0x0A => Instruction::LoadAFrom(Address8::BC),
            0x1A => Instruction::LoadAFrom(Address8::DE),
            0x2A => Instruction::LoadAFrom(Address8::HLInc),
            0x3A => Instruction::LoadAFrom(Address8::HLDec),
            0xE0 => Instruction::StoreATo(Address8::HighImmediate(self.fetch8(bus))),
            0xF0 => Instruction::LoadAFrom(Address8::HighImmediate(self.fetch8(bus))),
            0xE2 => Instruction::StoreATo(Address8::HighC),
            0xF2 => Instruction::LoadAFrom(Address8::HighC),
            0xEA => Instruction::StoreATo(Address8::Immediate(self.fetch16(bus))),
            0xFA => Instruction::LoadAFrom(Address8::Immediate(self.fetch16(bus))),

            // 00ddd100 / 00ddd101: INC/DEC r8
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                Instruction::Inc8(Operand8::from_bits(code >> 3))
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                Instruction::Dec8(Operand8::from_bits(code >> 3))
            }

            // 00pp0011/1011/1001: 16-bit increments, decrements, ADD HL
            0x03 | 0x13 | 0x23 | 0x33 => Instruction::Inc16(pair(code >> 4)),
            0x0B | 0x1B | 0x2B | 0x3B => Instruction::Dec16(pair(code >> 4)),
            0x09 | 0x19 | 0x29 | 0x39 => Instruction::AddHL(pair(code >> 4)),

            // ALU operations
            0x80..=0xBF => Instruction::Alu {
                op: AluOp::from_bits(code >> 3),
                src: Source8::Operand(Operand8::from_bits(code)),
            },
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => Instruction::Alu {
                op: AluOp::from_bits(code >> 3),
                src: Source8::Immediate(self.fetch8(bus)),
            },

            0x07 => Instruction::RotateA(RotateKind::Rlc),
            0x0F => Instruction::RotateA(RotateKind::Rrc),
            0x17 => Instruction::RotateA(RotateKind::Rl),
            0x1F => Instruction::RotateA(RotateKind::Rr),

            0xC3 => Instruction::Jump {
                condition: None,
                address: self.fetch16(bus),
            },
            0xC2 | 0xCA | 0xD2 | 0xDA => Instruction::Jump {
                condition: Some(Condition::from_bits(code >> 3)),
                address: self.fetch16(bus),
            },
            0xE9 => Instruction::JumpHL,
            0x18 => Instruction::JumpRelative {
                condition: None,
                offset: self.fetch8(bus) as i8,
            },
            0x20 | 0x28 | 0x30 | 0x38 => Instruction::JumpRelative {
                condition: Some(Condition::from_bits(code >> 3)),
                offset: self.fetch8(bus) as i8,
            },
            0xCD => Instruction::Call {
                condition: None,
                address: self.fetch16(bus),
            },
            0xC4 | 0xCC | 0xD4 | 0xDC => Instruction::Call {
                condition: Some(Condition::from_bits(code >> 3)),
                address: self.fetch16(bus),
            },
            0xC9 => Instruction::Return { condition: None },
            0xC0 | 0xC8 | 0xD0 | 0xD8 => Instruction::Return {
                condition: Some(Condition::from_bits(code >> 3)),
            },
            0xD9 => Instruction::ReturnInterrupt,
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                Instruction::Reset(u16::from(code & 0x38))
            }

            0x08 => Instruction::StoreStackPointer(self.fetch16(bus)),
            0xC5 | 0xD5 | 0xE5 | 0xF5 => Instruction::Push(stack_pair(code >> 4)),
            0xC1 | 0xD1 | 0xE1 | 0xF1 => Instruction::Pop(stack_pair(code >> 4)),
            0xF9 => Instruction::LoadStackPointerHl,

            0xF3 => Instruction::DisableInterrupts,

            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                return Err(CpuError::Illegal { pc, opcode: code });
            }

            _ => {
                return Err(CpuError::Unimplemented {
                    pc,
                    opcode: Opcode::Base(code),
                });
            }
        };

        Ok(instruction)
    }
}

fn pair(bits: u8) -> Register16 {
    match bits & 3 {
        0 => Register16::BC,
        1 => Register16::DE,
        2 => Register16::HL,
        3 => Register16::SP,
        _ => unreachable!(),
    }
}

fn stack_pair(bits: u8) -> Register16 {
    match bits & 3 {
        0 => Register16::BC,
        1 => Register16::DE,
        2 => Register16::HL,
        3 => Register16::AF,
        _ => unreachable!(),
    }
}

fn cb_instruction(code: u8) -> Instruction {
    let target = Operand8::from_bits(code);
    let group = (code >> 3) & 7;
    let op = match code >> 6 {
        0 => match group {
            0 => CbOperation::Rotate(RotateKind::Rlc),
            1 => CbOperation::Rotate(RotateKind::Rrc),
            2 => CbOperation::Rotate(RotateKind::Rl),
            3 => CbOperation::Rotate(RotateKind::Rr),
            4 => CbOperation::Sla,
            5 => CbOperation::Sra,
            6 => CbOperation::Swap,
            7 => CbOperation::Srl,
            _ => unreachable!(),
        },
        1 => CbOperation::Bit(group),
        2 => CbOperation::Res(group),
        3 => CbOperation::Set(group),
        _ => unreachable!(),
    };
    Instruction::Cb { op, target }
}
