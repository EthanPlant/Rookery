use crate::{AluOp, CbOperation, FLAG_C, FLAG_H, FLAG_N, FLAG_Z, RotateKind};

pub(crate) fn accumulator(op: AluOp, a: u8, old_f: u8, rhs: u8) -> (u8, u8) {
    match op {
        AluOp::Add | AluOp::Adc => {
            let carry = u8::from(op == AluOp::Adc && old_f & FLAG_C != 0);
            let sum = u16::from(a) + u16::from(rhs) + u16::from(carry);
            let result = sum as u8;
            let mut flags = if result == 0 { FLAG_Z } else { 0 };
            if (a & 0x0F) + (rhs & 0x0F) + (carry) > 0x0F {
                flags |= FLAG_H;
            }
            if sum > 0xFF {
                flags |= FLAG_C;
            }
            (result, flags)
        }
        AluOp::Sub | AluOp::Sbc | AluOp::Cp => {
            let borrow = u8::from(op == AluOp::Sbc && old_f & FLAG_C != 0);
            let rhs_wide = u16::from(rhs) + u16::from(borrow);
            let result = a.wrapping_sub(rhs).wrapping_sub(borrow);
            let mut flags = FLAG_N;
            if result == 0 {
                flags |= FLAG_Z;
            }
            if (a & 0x0F) < (rhs & 0x0F) + borrow {
                flags |= FLAG_H;
            }
            if u16::from(a) < rhs_wide {
                flags |= FLAG_C;
            }
            (if op == AluOp::Cp { a } else { result }, flags)
        }
        AluOp::And => {
            let result = a & rhs;
            (result, FLAG_H | if result == 0 { FLAG_Z } else { 0 })
        }
        AluOp::Xor => {
            let result = a ^ rhs;
            (result, if result == 0 { FLAG_Z } else { 0 })
        }
        AluOp::Or => {
            let result = a | rhs;
            (result, if result == 0 { FLAG_Z } else { 0 })
        }
    }
}

pub(crate) fn inc8(value: u8, old_f: u8) -> (u8, u8) {
    let result = value.wrapping_add(1);
    let mut flags = old_f & FLAG_C;
    if result == 0 {
        flags |= FLAG_Z;
    }
    if value & 0x0F == 0x0F {
        flags |= FLAG_H
    }
    (result, flags)
}

pub(crate) fn dec8(value: u8, old_f: u8) -> (u8, u8) {
    let result = value.wrapping_sub(1);
    let mut flags = (old_f & FLAG_C) | FLAG_N;
    if result == 0 {
        flags |= FLAG_Z;
    }
    if value & 0x0F == 0 {
        flags |= FLAG_H;
    }
    (result, flags)
}

pub(crate) fn add_hl(hl: u16, rhs: u16, old_f: u8) -> (u16, u8) {
    let result = hl.wrapping_add(rhs);
    let mut flags = old_f & FLAG_Z;
    if (hl & 0x0FFF) + (rhs & 0x0FFF) > 0x0FFF {
        flags |= FLAG_H;
    }
    if u32::from(hl) + u32::from(rhs) > 0xFFFF {
        flags |= FLAG_C;
    }
    (result, flags)
}

pub(crate) fn offset_sp(sp: u16, offset: i16) -> (u16, u8) {
    let result = sp.wrapping_add_signed(offset);
    let mut flags = 0;
    if (sp & 0x0F) + (offset as u16 & 0x0F) > 0x0F {
        flags |= FLAG_H;
    }
    if (sp & 0xFF) + (offset as u16 & 0xFF) > 0xFF {
        flags |= FLAG_C;
    }
    (result, flags)
}

pub(crate) fn rotate(value: u8, old_f: u8, kind: RotateKind, set_z: bool) -> (u8, u8) {
    let carry_in = u8::from(old_f & FLAG_C != 0);
    let (result, carry_out) = match kind {
        RotateKind::Rlc => (value.rotate_left(1), value & 0x80 != 0),
        RotateKind::Rrc => (value.rotate_right(1), value & 0x01 != 0),
        RotateKind::Rl => ((value << 1) | carry_in, value & 0x80 != 0),
        RotateKind::Rr => ((value >> 1) | (carry_in << 7), value & 0x01 != 0),
    };
    let mut flags = if carry_out { FLAG_C } else { 0 };
    if set_z && result == 0 {
        flags |= FLAG_Z;
    }
    (result, flags)
}

pub(crate) fn shift(value: u8, op: CbOperation) -> (u8, u8) {
    let (result, carry_out) = match op {
        CbOperation::Sla => (value << 1, value & 0x80 != 0),
        CbOperation::Sra => (value >> 1 | (value & 0x80), value & 0x01 != 0),
        CbOperation::Swap => (value.rotate_left(4), false),
        CbOperation::Srl => (value >> 1, value & 0x01 != 0),
        _ => unreachable!("not a shift/swap operation"),
    };
    let flags = (if result == 0 { FLAG_Z } else { 0 }) | (if carry_out { FLAG_C } else { 0 });
    (result, flags)
}
