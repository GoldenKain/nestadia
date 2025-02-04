mod ines_header;
mod mapper_000;
mod mapper_001;
mod mapper_002;
mod mapper_003;
mod mapper_004;
mod mapper_066;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::TryFrom as _;

use self::ines_header::{Flags6, INesHeader};
use self::mapper_000::Mapper000;
use self::mapper_001::Mapper001;
use self::mapper_002::Mapper002;
use self::mapper_003::Mapper003;
use self::mapper_004::Mapper004;
use self::mapper_066::Mapper066;

#[derive(Debug, Clone, Copy)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
    OneScreenLower,
    OneScreenUpper,
}

#[derive(Debug, Clone, Copy)]
pub enum RomParserError {
    TooShort,
    InvalidMagicBytes,
    MapperNotImplemented,
}

impl core::fmt::Display for RomParserError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{:?}", &self)
    }
}

enum CartridgeReadTarget {
    PrgRam(u8),
    PrgRom(usize),
}

trait Mapper: Send + Sync {
    fn cpu_map_read(&self, addr: u16) -> CartridgeReadTarget;
    fn cpu_map_write(&mut self, addr: u16, data: u8);
    fn ppu_map_read(&mut self, addr: u16) -> usize; // This is mutable because of side effects on some mapper that serves as a scanline counter
    fn ppu_map_write(&self, addr: u16) -> Option<usize>;
    fn mirroring(&self) -> Mirroring;
    fn get_sram(&self) -> Option<&[u8]>;

    fn irq_state(&self) -> bool {
        false
    }
    fn irq_clear(&mut self) {}

    #[cfg(feature = "debugger")]
    fn get_prg_bank(&self, addr: u16) -> Option<u8>;
}

pub struct Cartridge {
    chr_ram: bool,
    prg_memory: Vec<u8>, // program ROM, used by CPU
    chr_memory: Vec<u8>, // character ROM, used by PPU
    mapper: Box<dyn Mapper>,
}

impl Cartridge {
    pub fn load(rom: &[u8], save_data: Option<&[u8]>) -> Result<Self, RomParserError> {
        const PRG_BANK_SIZE: usize = 16384;
        const CHR_BANK_SIZE: usize = 8192;

        let header: INesHeader = INesHeader::try_from(rom)?;

        log::info!("ROM info: {:?}", &header);

        let mirroring = if header.flags6.contains(Flags6::FOUR_SCREEN) {
            Mirroring::FourScreen
        } else if header.flags6.contains(Flags6::MIRRORING) {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };

        let mapper: Box<dyn Mapper> = match header.mapper_id {
            0 => Box::new(Mapper000::new(header.prg_size, mirroring)),
            1 => Box::new(Mapper001::new(header.prg_size, mirroring, save_data)),
            2 => Box::new(Mapper002::new(header.prg_size, mirroring)),
            3 => Box::new(Mapper003::new(header.prg_size, mirroring)),
            4 => Box::new(Mapper004::new(header.prg_size, mirroring)),
            66 => Box::new(Mapper066::new(mirroring)),
            _ => return Err(RomParserError::MapperNotImplemented),
        };

        let chr_memory_len = CHR_BANK_SIZE * (header.chr_size as usize);
        let prg_memory_len = PRG_BANK_SIZE * (header.prg_size as usize);

        let prg_start = if header.flags6.contains(Flags6::TRAINER) {
            512 + 16
        } else {
            16
        };

        let expected_rom_size = prg_start + prg_memory_len + chr_memory_len;
        if rom.len() < expected_rom_size {
            log::error!(
                "Invalid ROM size: expected {} bytes of memory, but ROM has {}",
                expected_rom_size,
                rom.len()
            );
            return Err(RomParserError::TooShort);
        }

        // PRG memory
        let prg_end = prg_start + prg_memory_len;
        let prg_memory = rom[prg_start..prg_end].to_vec();
        assert_eq!(prg_memory.len(), prg_memory_len);

        // CHR memory
        // Don't parse if it's RAM
        let chr_ram = header.chr_size == 0;
        let chr_memory = if !chr_ram {
            let chr_start = prg_end;
            let chr_end = prg_end + chr_memory_len;
            rom[chr_start..chr_end].to_vec()
        } else {
            vec![0u8; CHR_BANK_SIZE]
        };

        Ok(Cartridge {
            chr_ram,
            prg_memory,
            chr_memory,
            mapper,
        })
    }

    pub fn mirroring(&self) -> Mirroring {
        self.mapper.mirroring()
    }

    pub fn read_prg_mem(&self, addr: u16) -> u8 {
        match self.mapper.cpu_map_read(addr) {
            CartridgeReadTarget::PrgRom(rom_addr) => {
                self.prg_memory[rom_addr % self.prg_memory.len()]
            }
            CartridgeReadTarget::PrgRam(data) => data,
        }
    }

    pub fn write_prg_mem(&mut self, addr: u16, data: u8) {
        self.mapper.cpu_map_write(addr, data);
    }

    pub fn read_chr_mem(&mut self, addr: u16) -> u8 {
        let addr = self.mapper.ppu_map_read(addr);
        self.chr_memory[addr % self.chr_memory.len()]
    }

    pub fn write_chr_mem(&mut self, addr: u16, data: u8) {
        if self.chr_ram {
            if let Some(addr) = self.mapper.ppu_map_write(addr) {
                self.chr_memory[addr] = data;
            } else {
                log::warn!(
                    "attempted to write on CHR memory at {}, but this is not supported by this mapper",
                    addr
                );
            }
        } else {
            log::warn!(
                "attempted to write on CHR memory at {}, but this ROM uses CHR ROM",
                addr
            );
        };
    }

    pub fn get_save_data(&self) -> Option<&[u8]> {
        self.mapper.get_sram()
    }

    pub fn take_irq_set_state(&mut self) -> bool {
        let state = self.mapper.irq_state();
        self.mapper.irq_clear();
        state
    }

    #[cfg(feature = "debugger")]
    pub fn get_prg_bank(&self, addr: u16) -> Option<u8> {
        self.mapper.get_prg_bank(addr)
    }
}
