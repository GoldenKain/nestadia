#[cfg(feature = "debugger")]
pub mod disassembler;
mod opcode;

use std::convert::TryFrom as _;

use bitflags::bitflags;

use self::opcode::Opcode;
use crate::bus::CpuBus;
use crate::ExecutionMode;

const STACK_BASE: u16 = 0x0100;
const PC_START: u16 = 0xFFFC;
const IRQ_HANDLER: u16 = 0xFFFE;
#[allow(unused_variables)] // FIXME
const NMI_HANDLER: u16 = 0xFFFA;

#[cfg(not(feature = "true-flags"))]
const FLAG3: &[u8] = include_bytes!("../../../flags/flag3-debug.txt");

#[cfg(feature = "true-flags")]
const FLAG3: &[u8] = include_bytes!("../../../flags/flag3-prod.txt");

#[cfg(not(feature = "true-flags"))]
const FLAG4: &[u8] = include_bytes!("../../../flags/flag4-debug.txt");

#[cfg(feature = "true-flags")]
const FLAG4: &[u8] = include_bytes!("../../../flags/flag4-prod.txt");

bitflags! {
    pub struct StatusRegister: u8 {
        const C = (1 << 0);
        const Z = (1 << 1);
        const I = (1 << 2);
        const D = (1 << 3);
        const B = (1 << 4);
        const U = (1 << 5);
        const V = (1 << 6);
        const N = (1 << 7);
    }
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub st: u8,
    pub pc: u16,
    pub cycles: u8,
    pub status_register: StatusRegister,

    execution_mode: ExecutionMode,
}

impl Cpu {
    pub fn new(execution_mode: ExecutionMode) -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            st: 0,
            pc: 0,
            cycles: 0,
            status_register: StatusRegister::empty(),
            execution_mode,
        }
    }

    pub fn reset(&mut self, bus: &mut CpuBus<'_>) {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.st = 0xFD;
        self.cycles = 8;
        self.status_register = StatusRegister::empty() | StatusRegister::U;
        self.pc = u16::from(bus.read(PC_START)) | (u16::from(bus.read(PC_START + 1)) << 8);
    }

    #[allow(dead_code)] // FIXME
    pub fn irq(&mut self, bus: &mut CpuBus<'_>) {
        if !self.status_register.contains(StatusRegister::I) {
            // Push current PC
            self.stack_push(bus, ((self.pc >> 8) & 0xff) as u8);
            self.stack_push(bus, (self.pc & 0xff) as u8);

            // Push status register
            self.status_register.set(StatusRegister::B, false);
            self.status_register.set(StatusRegister::U, true);
            self.stack_push(bus, self.status_register.bits());

            self.status_register.set(StatusRegister::I, true);

            self.pc =
                u16::from(bus.read(IRQ_HANDLER)) | (u16::from(bus.read(IRQ_HANDLER + 1)) << 8);

            self.cycles = 7;
        }
    }

    #[allow(dead_code)] // FIXME
    pub fn nmi(&mut self, bus: &mut CpuBus<'_>) {
        // Push current PC
        self.stack_push(bus, ((self.pc >> 8) & 0xff) as u8);
        self.stack_push(bus, (self.pc & 0xff) as u8);

        // Push status register
        self.status_register.set(StatusRegister::B, false);
        self.status_register.set(StatusRegister::U, true);
        self.stack_push(bus, self.status_register.bits());

        self.status_register.set(StatusRegister::I, true);

        self.pc = u16::from(bus.read(NMI_HANDLER))
            | (u16::from(bus.read(NMI_HANDLER.wrapping_add(1))) << 8);

        self.cycles = 7;
    }

    pub fn clock(&mut self, bus: &mut CpuBus<'_>) {
        if self.cycles == 0 {
            let opcode = match Opcode::try_from(bus.read(self.pc)) {
                Ok(o) => o,
                Err(_) => {
                    log::warn!("Unknown opcode {}, treating as a NOP...", bus.read(self.pc));
                    Opcode::Nop
                }
            };
            self.pc = self.pc.wrapping_add(1);

            match &opcode {
                Opcode::Brk => {
                    self.inst_brk(bus);
                }
                Opcode::OraIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_ora(op)
                }
                Opcode::OraZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_ora(op)
                }
                Opcode::AslZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_asl(op);
                    bus.write(addr, result);
                }
                Opcode::Php => {
                    self.inst_php(bus);
                }
                Opcode::OraImm => {
                    let op = self.am_imm(bus);
                    self.inst_ora(op);
                }
                Opcode::AslAcc => {
                    self.a = self.inst_asl(self.a);
                }
                Opcode::OraAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_ora(op);
                }
                Opcode::AslAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_asl(op);
                    bus.write(addr, result);
                }

                Opcode::Bpl => {
                    let addr = self.am_rel(bus);
                    self.inst_bpl(addr);
                }
                Opcode::OraIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_ora(op);
                }
                Opcode::OraZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_ora(op);
                }
                Opcode::AslZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_asl(op);
                    bus.write(addr, result);
                }
                Opcode::Clc => {
                    self.inst_clc();
                }
                Opcode::OraAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_ora(op);
                }
                Opcode::OraAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_ora(op);
                }
                Opcode::AslAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_asl(op);
                    bus.write(addr, result);
                }

                Opcode::JsrAbs => {
                    let addr = self.am_abs(bus);
                    self.inst_jsr(bus, addr);
                }
                Opcode::AndIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::BitZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_bit(op);
                }
                Opcode::AndZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::RolZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_rol(op);
                    bus.write(addr, result);
                }
                Opcode::Plp => {
                    self.inst_plp(bus);
                }
                Opcode::AndImm => {
                    let op = self.am_imm(bus);
                    self.inst_and(op);
                }
                Opcode::RolAcc => {
                    self.a = self.inst_rol(self.a);
                }
                Opcode::BitAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_bit(op);
                }
                Opcode::AndAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::RolAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_rol(op);
                    bus.write(addr, result);
                }

                Opcode::Bmi => {
                    let addr = self.am_rel(bus);
                    self.inst_bmi(addr);
                }
                Opcode::AndIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::AndZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::RolZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_rol(op);
                    bus.write(addr, result);
                }
                Opcode::Sec => {
                    self.inst_sec();
                }
                Opcode::AndAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::AndAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_and(op);
                }
                Opcode::RolAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_rol(op);
                    bus.write(addr, result);
                }

                Opcode::Rti => {
                    self.inst_rti(bus);
                }
                Opcode::EorIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::EorZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::LsrZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_lsr(op);
                    bus.write(addr, result);
                }
                Opcode::Pha => {
                    self.inst_pha(bus);
                }
                Opcode::EorImm => {
                    let op = self.am_imm(bus);
                    self.inst_eor(op);
                }
                Opcode::LsrAcc => {
                    self.a = self.inst_lsr(self.a);
                }
                Opcode::JmpAbs => {
                    let addr = self.am_abs(bus);
                    self.inst_jmp(addr);
                }
                Opcode::EorAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::LsrAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_lsr(op);
                    bus.write(addr, result);
                }

                Opcode::Bvc => {
                    let addr = self.am_rel(bus);
                    self.inst_bvc(addr);
                }
                Opcode::EorIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::EorZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::LsrZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_lsr(op);
                    bus.write(addr, result);
                }
                Opcode::Cli => {
                    self.inst_cli();
                }
                Opcode::EorAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::EorAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_eor(op);
                }
                Opcode::LsrAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_lsr(op);
                    bus.write(addr, result);
                }

                Opcode::Rts => {
                    self.inst_rts(bus);
                }
                Opcode::AdcIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::AdcZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::RorZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_ror(op);
                    bus.write(addr, result);
                }
                Opcode::Pla => {
                    self.inst_pla(bus);
                }
                Opcode::AdcImm => {
                    let op = self.am_imm(bus);
                    self.inst_adc(op);
                }
                Opcode::RorAcc => {
                    self.a = self.inst_ror(self.a);
                }
                Opcode::JmpInd => {
                    let addr = self.am_ind(bus);
                    self.inst_jmp(addr);
                }
                Opcode::AdcAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::RorAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_ror(op);
                    bus.write(addr, result);
                }

                Opcode::Bvs => {
                    let addr = self.am_rel(bus);
                    self.inst_bvs(addr);
                }
                Opcode::AdcIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::AdcZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::RorZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_ror(op);
                    bus.write(addr, result);
                }
                Opcode::Sei => {
                    self.inst_sei();
                }
                Opcode::AdcAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::AdcAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_adc(op);
                }
                Opcode::RorAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    self.inst_ror(op);
                }

                Opcode::FlagAcc => {
                    let flag = match self.execution_mode {
                        ExecutionMode::Ring0 => FLAG3,
                        ExecutionMode::Ring3 => FLAG4,
                    };

                    let index = (self.a as usize) % flag.len();
                    self.a = flag[index];
                }

                Opcode::StaIndX => {
                    let addr = self.am_izx(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::StyZp => {
                    let addr = self.am_zp(bus);
                    self.inst_sty(bus, addr);
                }
                Opcode::StaZp => {
                    let addr = self.am_zp(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::StxZp => {
                    let addr = self.am_zp(bus);
                    self.inst_stx(bus, addr);
                }
                Opcode::Dey => {
                    self.inst_dey();
                }
                Opcode::Txa => {
                    self.inst_txa();
                }
                Opcode::StyAbs => {
                    let addr = self.am_abs(bus);
                    self.inst_sty(bus, addr);
                }
                Opcode::StaAbs => {
                    let addr = self.am_abs(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::StxAbs => {
                    let addr = self.am_abs(bus);
                    self.inst_stx(bus, addr);
                }

                Opcode::Bcc => {
                    let addr = self.am_rel(bus);
                    self.inst_bcc(addr);
                }
                Opcode::StaIndY => {
                    let (addr, _) = self.am_izy(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::StyZpX => {
                    let addr = self.am_zpx(bus);
                    self.inst_sty(bus, addr);
                }
                Opcode::StaZpX => {
                    let addr = self.am_zpx(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::StxZpY => {
                    let addr = self.am_zpy(bus);
                    self.inst_stx(bus, addr);
                }
                Opcode::Tya => {
                    self.inst_tya();
                }
                Opcode::StaAbsY => {
                    let (addr, _) = self.am_aby(bus);
                    self.inst_sta(bus, addr);
                }
                Opcode::Txs => {
                    self.inst_txs();
                }
                Opcode::StaAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    self.inst_sta(bus, addr);
                }

                Opcode::LdyImm => {
                    let op = self.am_imm(bus);
                    self.inst_ldy(op);
                }
                Opcode::LdaIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdxImm => {
                    let op = self.am_imm(bus);
                    self.inst_ldx(op);
                }
                Opcode::LdyZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_ldy(op);
                }
                Opcode::LdaZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdxZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_ldx(op);
                }
                Opcode::Tay => {
                    self.inst_tay();
                }
                Opcode::LdaImm => {
                    let op = self.am_imm(bus);
                    self.inst_lda(op);
                }
                Opcode::Tax => {
                    self.inst_tax();
                }
                Opcode::LdyAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_ldy(op);
                }
                Opcode::LdaAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdxAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_ldx(op);
                }

                Opcode::Bcs => {
                    let offset = self.am_rel(bus);
                    self.inst_bcs(offset);
                }
                Opcode::LdaIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdyZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_ldy(op);
                }
                Opcode::LdaZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdxZpY => {
                    let addr = self.am_zpy(bus);
                    let op = bus.read(addr);
                    self.inst_ldx(op);
                }
                Opcode::Clv => {
                    self.inst_clv();
                }
                Opcode::LdaAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::Tsx => {
                    self.inst_tsx();
                }
                Opcode::LdyAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_ldy(op);
                }
                Opcode::LdaAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_lda(op);
                }
                Opcode::LdxAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_ldx(op);
                }

                Opcode::CpyImm => {
                    let op = self.am_imm(bus);
                    self.inst_cpy(op);
                }
                Opcode::CmpIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::CpyZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_cpy(op);
                }
                Opcode::CmpZp => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::DecZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_dec(op);
                    bus.write(addr, result);
                }
                Opcode::Iny => {
                    self.inst_iny();
                }
                Opcode::CmpImm => {
                    let op = self.am_imm(bus);
                    self.inst_cmp(op);
                }
                Opcode::Dex => {
                    self.inst_dex();
                }
                Opcode::CpyAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_cpy(op);
                }
                Opcode::CmpAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::DecAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_dec(op);
                    bus.write(addr, result);
                }

                Opcode::Bne => {
                    let offset = self.am_rel(bus);
                    self.inst_bne(offset);
                }
                Opcode::CmpIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::CmpZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::DecZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_dec(op);
                    bus.write(addr, result);
                }
                Opcode::Cld => {
                    self.inst_cld();
                }
                Opcode::CmpAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::CmpAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_cmp(op);
                }
                Opcode::DecAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_dec(op);
                    bus.write(addr, result);
                }

                Opcode::CpxImm => {
                    let op = self.am_imm(bus);
                    self.inst_cpx(op);
                }
                Opcode::SbcIndX => {
                    let addr = self.am_izx(bus);
                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::CpxZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_cpx(op);
                }
                Opcode::SbcZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::IncZp => {
                    let addr = self.am_zp(bus);
                    let op = bus.read(addr);
                    let result = self.inst_inc(op);
                    bus.write(addr, result);
                }
                Opcode::Inx => {
                    self.inst_inx();
                }
                Opcode::SbcImm => {
                    let op = self.am_imm(bus);
                    self.inst_sbc(op);
                }
                Opcode::Nop => {
                    // This is intended, a NOP actually does nothing.
                }
                Opcode::CpxAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_cpx(op);
                }
                Opcode::SbcAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::IncAbs => {
                    let addr = self.am_abs(bus);
                    let op = bus.read(addr);
                    let result = self.inst_inc(op);
                    bus.write(addr, result);
                }

                Opcode::Beq => {
                    let offset = self.am_rel(bus);
                    self.inst_beq(offset);
                }
                Opcode::SbcIndY => {
                    let (addr, extra_cycle) = self.am_izy(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::SbcZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::IncZpX => {
                    let addr = self.am_zpx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_inc(op);
                    bus.write(addr, result);
                }
                Opcode::Sed => {
                    self.inst_sed();
                }
                Opcode::SbcAbsY => {
                    let (addr, extra_cycle) = self.am_aby(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::SbcAbsX => {
                    let (addr, extra_cycle) = self.am_abx(bus);
                    if extra_cycle {
                        self.cycles += 1;
                    }

                    let op = bus.read(addr);
                    self.inst_sbc(op);
                }
                Opcode::IncAbsX => {
                    let (addr, _) = self.am_abx(bus);
                    let op = bus.read(addr);
                    let result = self.inst_inc(op);
                    bus.write(addr, result);
                }
            };

            self.cycles += opcode.cycles();
        }
        self.cycles -= 1;
    }

    // Addressing modes
    fn am_imm(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        self.pc = self.pc.wrapping_add(1);
        bus.read(self.pc.wrapping_sub(1))
    }

    fn am_zp(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(1);
        (u16::from(bus.read(self.pc.wrapping_sub(1)))) & 0x00ff
    }

    fn am_zpx(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(1);
        u16::from(bus.read(self.pc.wrapping_sub(1)).wrapping_add(self.x)) & 0x00ff
    }

    fn am_zpy(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(1);
        u16::from(bus.read(self.pc.wrapping_sub(1)).wrapping_add(self.y)) & 0x00ff
    }

    fn am_abs(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(2);
        (u16::from(bus.read(self.pc.wrapping_sub(2))))
            | (u16::from(bus.read(self.pc.wrapping_sub(1))) << 8)
    }

    fn am_abx(&mut self, bus: &mut CpuBus<'_>) -> (u16, bool) {
        self.pc = self.pc.wrapping_add(2);
        let address_no_offset = (u16::from(bus.read(self.pc.wrapping_sub(2))))
            | (u16::from(bus.read(self.pc.wrapping_sub(1))) << 8);
        let address_with_offset = address_no_offset.wrapping_add(u16::from(self.x));

        // Check if page has changed and request additionnal clock cycle
        let need_additionnal_cycle = address_no_offset & 0xff00 != address_with_offset & 0xff00;

        (address_with_offset, need_additionnal_cycle)
    }

    fn am_aby(&mut self, bus: &mut CpuBus<'_>) -> (u16, bool) {
        self.pc = self.pc.wrapping_add(2);
        let address_no_offset = (u16::from(bus.read(self.pc.wrapping_sub(2))))
            | (u16::from(bus.read(self.pc.wrapping_sub(1))) << 8);
        let address_with_offset = address_no_offset.wrapping_add(u16::from(self.y));

        // Check if page has changed and request additionnal clock cycle
        let need_additionnal_cycle = address_no_offset & 0xff00 != address_with_offset & 0xff00;

        (address_with_offset, need_additionnal_cycle)
    }

    fn am_ind(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(2);

        let ptr = (u16::from(bus.read(self.pc.wrapping_sub(2))))
            | (u16::from(bus.read(self.pc.wrapping_sub(1))) << 8);

        if ptr | 0x00ff == 0x00ff {
            // Simutate undefinied behavior at page end. The page is not updated.
            u16::from(bus.read(ptr)) | (u16::from(bus.read(ptr & 0xff00)) << 8)
        } else {
            u16::from(bus.read(ptr)) | (u16::from(bus.read(ptr.wrapping_add(1))) << 8)
        }
    }

    fn am_izx(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(1);
        let ptr = u16::from(bus.read(self.pc.wrapping_sub(1)).wrapping_add(self.x)) & 0x00ff;

        (u16::from(bus.read(ptr))) | (u16::from(bus.read(ptr.wrapping_add(1) & 0x00ff)) << 8)
    }

    fn am_izy(&mut self, bus: &mut CpuBus<'_>) -> (u16, bool) {
        self.pc = self.pc.wrapping_add(1);
        let ptr = (u16::from(bus.read(self.pc.wrapping_sub(1)))) & 0x00ff;

        let address_no_offset =
            (u16::from(bus.read(ptr))) | (u16::from(bus.read(ptr.wrapping_add(1))) << 8);

        let address_with_offset = address_no_offset.wrapping_add(u16::from(self.y));

        // Check if page has changed and request additionnal clock cycle
        let need_additionnal_cycle = address_no_offset & 0xff00 != address_with_offset & 0xff00;

        (address_with_offset, need_additionnal_cycle)
    }

    fn am_rel(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        self.pc = self.pc.wrapping_add(1);

        let address = bus.read(self.pc.wrapping_sub(1));

        // Sign expansion
        if address & 0x80 == 0x80 {
            (u16::from(address)) | 0xff00
        } else {
            u16::from(address)
        }
    }

    // Instructions
    fn inst_adc(&mut self, op: u8) {
        #![allow(clippy::many_single_char_names)]

        let mut result: u16 = (u16::from(self.a)).wrapping_add(u16::from(op));

        if self.status_register.contains(StatusRegister::C) {
            result = result.wrapping_add(1);
        };

        let c = result > 0xff;
        self.status_register.set(StatusRegister::C, c);

        let r = (result & 0xff) as u8;

        let v = ((self.a ^ r) & !(self.a ^ op)) & 0x80 == 0x80;
        self.status_register.set(StatusRegister::C, v);

        self.a = r;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & 0x80 == 0x80;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_and(&mut self, op: u8) {
        self.a &= op;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & 0x80 == 0x80;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_asl(&mut self, op: u8) -> u8 {
        self.status_register
            .set(StatusRegister::C, op & 0x80 == 0x80);
        let result = op << 1;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & 0x80 == 0x80;
        self.status_register.set(StatusRegister::N, n);

        result
    }

    fn inst_bcc(&mut self, offset: u16) {
        if !self.status_register.contains(StatusRegister::C) {
            self.branch(offset);
        }
    }

    fn inst_bcs(&mut self, offset: u16) {
        if self.status_register.contains(StatusRegister::C) {
            self.branch(offset);
        }
    }

    fn inst_beq(&mut self, offset: u16) {
        if self.status_register.contains(StatusRegister::Z) {
            self.branch(offset);
        }
    }

    fn inst_bit(&mut self, op: u8) {
        let result = self.a & op;

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::V, result & (1 << 6) > 0);
        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
    }

    fn inst_bmi(&mut self, offset: u16) {
        if self.status_register.contains(StatusRegister::N) {
            self.branch(offset);
        }
    }

    fn inst_bne(&mut self, offset: u16) {
        if !self.status_register.contains(StatusRegister::Z) {
            self.branch(offset);
        }
    }

    fn inst_bpl(&mut self, offset: u16) {
        if !self.status_register.contains(StatusRegister::N) {
            self.branch(offset);
        }
    }

    fn inst_brk(&mut self, bus: &mut CpuBus<'_>) {
        // Adds 1 to PC so we return to the next instruction
        self.pc = self.pc.wrapping_add(1);

        // Push current PC
        self.stack_push(bus, ((self.pc >> 8) & 0xff) as u8);
        self.stack_push(bus, (self.pc & 0xff) as u8);

        // Push status register
        self.status_register.set(StatusRegister::B, true);
        self.status_register.set(StatusRegister::U, true);
        self.stack_push(bus, self.status_register.bits());

        self.status_register.set(StatusRegister::I, true);

        self.pc = u16::from(bus.read(IRQ_HANDLER))
            | (u16::from(bus.read(IRQ_HANDLER.wrapping_add(1))) << 8);
    }

    fn inst_bvc(&mut self, offset: u16) {
        if !self.status_register.contains(StatusRegister::V) {
            self.branch(offset);
        }
    }

    fn inst_bvs(&mut self, offset: u16) {
        if self.status_register.contains(StatusRegister::V) {
            self.branch(offset);
        }
    }

    fn inst_clc(&mut self) {
        self.status_register.set(StatusRegister::C, false);
    }

    fn inst_cld(&mut self) {
        self.status_register.set(StatusRegister::D, false);
    }

    fn inst_cli(&mut self) {
        self.status_register.set(StatusRegister::I, false);
    }

    fn inst_clv(&mut self) {
        self.status_register.set(StatusRegister::V, false);
    }

    fn inst_cmp(&mut self, op: u8) {
        let result = self.a.wrapping_sub(op);

        let c = self.a >= op;
        self.status_register.set(StatusRegister::C, c);

        self.status_register.set(StatusRegister::Z, result == 0);

        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
    }

    fn inst_cpx(&mut self, op: u8) {
        let result = self.x.wrapping_sub(op);

        let c = self.x >= op;
        self.status_register.set(StatusRegister::C, c);

        self.status_register.set(StatusRegister::Z, result == 0);

        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
    }

    fn inst_cpy(&mut self, op: u8) {
        let result = self.y.wrapping_sub(op);

        let c = self.y >= op;
        self.status_register.set(StatusRegister::C, c);

        self.status_register.set(StatusRegister::Z, result == 0);

        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
    }

    fn inst_dec(&mut self, op: u8) -> u8 {
        let result = op.wrapping_sub(1);

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
        result
    }

    fn inst_dex(&mut self) {
        self.x = self.x.wrapping_sub(1);

        let z = self.x == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.x & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_dey(&mut self) {
        self.y = self.y.wrapping_sub(1);

        let z = self.y == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.y & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_eor(&mut self, op: u8) {
        self.a ^= op;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_inc(&mut self, op: u8) -> u8 {
        let result = op.wrapping_add(1);

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);
        result
    }

    fn inst_inx(&mut self) {
        self.x = self.x.wrapping_add(1);

        let z = self.x == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.x & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_iny(&mut self) {
        self.y = self.y.wrapping_add(1);

        let z = self.y == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.y & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_jmp(&mut self, address: u16) {
        self.pc = address;
    }

    fn inst_jsr(&mut self, bus: &mut CpuBus<'_>, address: u16) {
        let pc = self.pc.wrapping_sub(1);

        self.stack_push(bus, (pc >> 8) as u8);
        self.stack_push(bus, (pc & 0x00ff) as u8);

        self.pc = address;
    }

    fn inst_lda(&mut self, op: u8) {
        self.a = op;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_ldx(&mut self, op: u8) {
        self.x = op;

        let z = self.x == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.x & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_ldy(&mut self, op: u8) {
        self.y = op;

        let z = self.y == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.y & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_lsr(&mut self, op: u8) -> u8 {
        self.status_register
            .set(StatusRegister::C, op & (1 << 0) > 0);
        let result = op >> 1;

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::N, result & (1 << 7) > 0);

        result
    }

    fn inst_ora(&mut self, op: u8) {
        self.a |= op;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_pha(&mut self, bus: &mut CpuBus<'_>) {
        self.stack_push(bus, self.a);
    }

    fn inst_php(&mut self, bus: &mut CpuBus<'_>) {
        self.status_register.set(StatusRegister::B, true);
        self.status_register.set(StatusRegister::U, true);

        self.stack_push(bus, self.status_register.bits());

        self.status_register.set(StatusRegister::B, false);
        self.status_register.set(StatusRegister::U, false);
    }

    fn inst_pla(&mut self, bus: &mut CpuBus<'_>) {
        self.a = self.stack_pop(bus);

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & 0x80 == 0x80;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_plp(&mut self, bus: &mut CpuBus<'_>) {
        self.status_register = StatusRegister::from_bits_truncate(self.stack_pop(bus));
        self.status_register.set(StatusRegister::B, false);
        self.status_register.set(StatusRegister::U, false);
    }

    fn inst_rol(&mut self, op: u8) -> u8 {
        let carry = self.status_register.contains(StatusRegister::C);

        self.status_register
            .set(StatusRegister::C, op & (1 << 7) > 0);

        let mut result = op << 1;

        if carry {
            result |= 1 << 0;
        }

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::Z, result & (1 << 7) > 0);

        result
    }

    fn inst_ror(&mut self, op: u8) -> u8 {
        let carry = self.status_register.contains(StatusRegister::C);

        self.status_register
            .set(StatusRegister::C, op & (1 << 0) > 0);

        let mut result = op >> 1;

        if carry {
            result |= 1 << 7;
        }

        self.status_register.set(StatusRegister::Z, result == 0);
        self.status_register
            .set(StatusRegister::Z, result & (1 << 7) > 0);

        result
    }

    fn inst_rti(&mut self, bus: &mut CpuBus<'_>) {
        self.status_register = StatusRegister::from_bits_truncate(self.stack_pop(bus));

        self.status_register.set(StatusRegister::B, false);
        self.status_register.set(StatusRegister::U, false);

        self.pc = u16::from(self.stack_pop(bus)) | (u16::from(self.stack_pop(bus)) << 8);
    }

    fn inst_rts(&mut self, bus: &mut CpuBus<'_>) {
        self.pc = u16::from(self.stack_pop(bus)) | (u16::from(self.stack_pop(bus)) << 8);
        self.pc = self.pc.wrapping_add(1);
    }

    fn inst_sbc(&mut self, op: u8) {
        let op = op ^ 0xff;
        self.inst_adc(op);
    }

    fn inst_sec(&mut self) {
        self.status_register.set(StatusRegister::C, true);
    }

    fn inst_sed(&mut self) {
        self.status_register.set(StatusRegister::D, true);
    }

    fn inst_sei(&mut self) {
        self.status_register.set(StatusRegister::I, true);
    }

    fn inst_sta(&mut self, bus: &mut CpuBus<'_>, address: u16) {
        bus.write(address, self.a);
    }

    fn inst_stx(&mut self, bus: &mut CpuBus<'_>, address: u16) {
        bus.write(address, self.x);
    }

    fn inst_sty(&mut self, bus: &mut CpuBus<'_>, address: u16) {
        bus.write(address, self.y);
    }

    fn inst_tax(&mut self) {
        self.x = self.a;

        let z = self.x == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.x & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_tay(&mut self) {
        self.y = self.a;

        let z = self.y == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.y & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_tsx(&mut self) {
        self.x = self.st;

        let z = self.x == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.x & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_txa(&mut self) {
        self.a = self.x;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    fn inst_txs(&mut self) {
        self.st = self.x;
    }

    fn inst_tya(&mut self) {
        self.a = self.y;

        let z = self.a == 0;
        self.status_register.set(StatusRegister::Z, z);

        let n = self.a & (1 << 7) > 0;
        self.status_register.set(StatusRegister::N, n);
    }

    // Other
    fn stack_push(&mut self, bus: &mut CpuBus<'_>, data: u8) {
        bus.write(STACK_BASE.wrapping_add(u16::from(self.st)), data);
        self.st = self.st.wrapping_sub(1);
    }

    fn stack_pop(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        self.st = self.st.wrapping_add(1);
        bus.read(STACK_BASE.wrapping_add(u16::from(self.st)))
    }

    fn branch(&mut self, offset: u16) {
        self.cycles += 1;

        let result = self.pc.wrapping_add(offset);

        // If there is a page change, it takes an extra cycle
        if (result & 0xff00) != (self.pc & 0xff00) {
            self.cycles += 1;
        };

        self.pc = result;
    }
}

impl CpuBus<'_> {
    fn write(&mut self, addr: u16, data: u8) {
        match addr {
            0..=0x1FFF => self.write_ram(addr, data),
            0x2000..=0x3fff => self.write_ppu_register(addr, data),
            0x4000..=0x4015 => (), // TODO: APU and Audio
            0x4016 => self.controller1_take_snapshot(),
            0x4017 => self.controller2_take_snapshot(),
            0x4018..=0x401f => (), // TODO: APU and Audio
            0x4020..=0xffff => self.write_prg_mem(addr, data),
        };
    }

    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0..=0x1FFF => self.read_ram(addr),
            0x2000..=0x3fff => self.read_ppu_register(addr),
            0x4000..=0x4015 => 0, // TODO: APU and Audio
            0x4016 => self.read_controller1_snapshot(),
            0x4017 => self.read_controller2_snapshot(),
            0x4018..=0x401f => 0, // TODO: APU and Audio
            0x4020..=0xffff => self.read_prg_mem(addr),
        }
    }
}
