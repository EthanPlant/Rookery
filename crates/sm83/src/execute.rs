use crate::{
    Address8, Bus, CbOperation, FLAG_C, FLAG_H, FLAG_Z, Instruction, Operand8, Sm83, Source8, alu,
};

impl Sm83 {
    pub(crate) fn execute<B: Bus>(&mut self, bus: &mut B, instruction: Instruction) -> u32 {
        match instruction {
            Instruction::Nop => 4,

            Instruction::Load8 { dst, src } => {
                let value = self.read_source8(bus, src);
                self.write_operand(bus, dst, value);
                match (dst.is_memory(), src.is_memory(), src.is_immediate()) {
                    (true, _, true) => 12,
                    (true, _, _) | (_, true, _) => 8,
                    (_, _, true) => 8,
                    _ => 4,
                }
            }
            Instruction::Load16 { dst, value } => {
                self.registers.set16(dst, value);
                12
            }
            Instruction::LoadAFrom(address) => {
                let effective = self.address8(address);
                self.registers.a = bus.read8(effective);
                self.finish_address8(address);
                match address {
                    Address8::Immediate(_) => 16,
                    Address8::HighImmediate(_) => 12,
                    _ => 8,
                }
            }
            Instruction::StoreATo(address) => {
                let effective = self.address8(address);
                bus.write8(effective, self.registers.a);
                self.finish_address8(address);
                match address {
                    Address8::Immediate(_) => 16,
                    Address8::HighImmediate(_) => 12,
                    _ => 8,
                }
            }

            Instruction::Inc8(target) => {
                let old = self.read_operand(bus, target);
                let (result, flags) = alu::inc8(old, self.registers.f);
                self.write_operand(bus, target, result);
                self.registers.f = flags;
                if target.is_memory() { 12 } else { 4 }
            }
            Instruction::Dec8(target) => {
                let old = self.read_operand(bus, target);
                let (result, flags) = alu::dec8(old, self.registers.f);
                self.write_operand(bus, target, result);
                self.registers.f = flags;
                if target.is_memory() { 12 } else { 4 }
            }
            Instruction::Inc16(register) => {
                self.registers
                    .set16(register, self.registers.get16(register).wrapping_add(1));
                8
            }
            Instruction::Dec16(register) => {
                self.registers
                    .set16(register, self.registers.get16(register).wrapping_sub(1));
                8
            }
            Instruction::AddHL(register) => {
                let (result, flags) = alu::add_hl(
                    self.registers.hl(),
                    self.registers.get16(register),
                    self.registers.f,
                );
                self.registers.set_hl(result);
                self.registers.f = flags;
                8
            }
            Instruction::Alu { op, src } => {
                let value = self.read_source8(bus, src);
                let (result, flags) =
                    alu::accumulator(op, self.registers.a, self.registers.f, value);
                self.registers.a = result;
                self.registers.f = flags;
                if src.is_memory() || src.is_immediate() {
                    8
                } else {
                    4
                }
            }

            Instruction::Jump { condition, address } => {
                let taken = condition.is_none_or(|c| c.evaluate(self.registers.f));
                if taken {
                    self.registers.pc = address;
                }
                if condition.is_none() || taken { 16 } else { 12 }
            }
            Instruction::JumpHL => {
                self.registers.pc = self.registers.hl();
                4
            }
            Instruction::JumpRelative { condition, offset } => {
                let taken = condition.is_none_or(|c| c.evaluate(self.registers.f));
                if taken {
                    self.registers.pc = self.registers.pc.wrapping_add_signed(i16::from(offset));
                }
                if taken { 12 } else { 8 }
            }
            Instruction::Call { condition, address } => {
                let taken = condition.is_none_or(|c| c.evaluate(self.registers.f));
                if taken {
                    self.push(bus, self.registers.pc);
                    self.registers.pc = address;
                    24
                } else {
                    12
                }
            }
            Instruction::Return { condition } => {
                let taken = condition.is_none_or(|c| c.evaluate(self.registers.f));
                if taken {
                    self.registers.pc = self.pop(bus);
                }
                match (condition, taken) {
                    (None, _) => 16,
                    (Some(_), true) => 20,
                    (Some(_), false) => 8,
                }
            }
            Instruction::ReturnInterrupt => {
                self.registers.pc = self.pop(bus);
                self.control.ime = true;
                16
            }
            Instruction::Reset(addr) => {
                self.push(bus, self.registers.pc);
                self.registers.pc = addr;
                16
            }

            Instruction::Push(register) => {
                self.push(bus, self.registers.get16(register));
                16
            }
            Instruction::Pop(register) => {
                let value = self.pop(bus);
                self.registers.set16(register, value);
                12
            }
            Instruction::StoreStackPointer(addr) => {
                bus.write8(addr, self.registers.sp as u8);
                bus.write8(addr.wrapping_add(1), (self.registers.sp >> 8) as u8);
                20
            }
            Instruction::LoadStackPointerHl => {
                self.registers.sp = self.registers.hl();
                8
            }
            Instruction::AddStackPointer(offset) => {
                let (result, flags) = alu::offset_sp(self.registers.sp, i16::from(offset));
                self.registers.sp = result;
                self.registers.f = flags;
                16
            }
            Instruction::StoreOffsetStackPointer(offset) => {
                let (result, flags) = alu::offset_sp(self.registers.sp, i16::from(offset));
                self.registers.set_hl(result);
                self.registers.f = flags;
                12
            }

            Instruction::RotateA(kind) => {
                let (result, flags) = alu::rotate(self.registers.a, self.registers.f, kind, false);
                self.registers.a = result;
                self.registers.f = flags;
                4
            }

            Instruction::Cb { op, target } => {
                let value = self.read_operand(bus, target);
                match op {
                    CbOperation::Bit(bit) => {
                        let flags = (self.registers.f & FLAG_C)
                            | FLAG_H
                            | if value & (1 << bit) == 0 { FLAG_Z } else { 0 };
                        self.registers.f = flags;
                        if target.is_memory() { 12 } else { 8 }
                    }
                    CbOperation::Set(bit) | CbOperation::Res(bit) => {
                        let mask = 1 << bit;
                        let result = if matches!(op, CbOperation::Set(_)) {
                            value | mask
                        } else {
                            value & !mask
                        };
                        self.write_operand(bus, target, result);
                        if target.is_memory() { 16 } else { 12 }
                    }
                    CbOperation::Rotate(kind) => {
                        let (result, flags) = alu::rotate(value, self.registers.f, kind, true);
                        self.write_operand(bus, target, result);
                        self.registers.f = flags;
                        if target.is_memory() { 16 } else { 8 }
                    }
                    CbOperation::Sla | CbOperation::Sra | CbOperation::Swap | CbOperation::Srl => {
                        let (result, flags) = alu::shift(value, op);
                        self.write_operand(bus, target, result);
                        self.registers.f = flags;
                        if target.is_memory() { 16 } else { 8 }
                    }
                }
            }

            Instruction::DisableInterrupts => {
                self.control.ime = false;
                self.control.ime_enable_delay = 0;
                4
            }
        }
    }

    fn read_operand<B: Bus>(&mut self, bus: &mut B, operand: Operand8) -> u8 {
        match operand {
            Operand8::Register(register) => self.registers.get8(register),
            Operand8::IndirectHL => bus.read8(self.registers.hl()),
        }
    }

    fn write_operand<B: Bus>(&mut self, bus: &mut B, operand: Operand8, value: u8) {
        match operand {
            Operand8::Register(register) => self.registers.set8(register, value),
            Operand8::IndirectHL => bus.write8(self.registers.hl(), value),
        }
    }

    fn read_source8<B: Bus>(&mut self, bus: &mut B, source: Source8) -> u8 {
        match source {
            Source8::Operand(operand) => self.read_operand(bus, operand),
            Source8::Immediate(value) => value,
        }
    }

    fn address8(&self, address: Address8) -> u16 {
        match address {
            Address8::BC => self.registers.bc(),
            Address8::DE => self.registers.de(),
            Address8::HLInc | Address8::HLDec => self.registers.hl(),
            Address8::HighC => 0xFF00 | u16::from(self.registers.c),
            Address8::HighImmediate(offset) => 0xFF00 | u16::from(offset),
            Address8::Immediate(value) => value,
        }
    }

    fn finish_address8(&mut self, address: Address8) {
        match address {
            Address8::HLInc => self.registers.set_hl(self.registers.hl().wrapping_add(1)),
            Address8::HLDec => self.registers.set_hl(self.registers.hl().wrapping_sub(1)),
            _ => {}
        }
    }
}
