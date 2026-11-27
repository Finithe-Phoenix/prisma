//! Fixed-address, freestanding Linux ELF64 executables. Only PT_LOAD bytes
//! enter the guest image. File offsets and guest addresses are separate.

use std::ops::Range;

use prisma_orchestrator::address_space::{AddressSpace, AddressSpaceError, Protection};
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_orchestrator::guest_layout::GuestLayout;
use prisma_orchestrator::guest_stack::{map_default_stack, DEFAULT_STACK_SIZE};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ElfLoadError {
    #[error("malformed ELF: {0}")]
    Invalid(&'static str),
    #[error("unsupported ELF: {0}")]
    Unsupported(&'static str),
}

use ElfLoadError::{Invalid, Unsupported};

struct Segment {
    address: Range<u64>,
    data: Range<usize>,
    protection: Protection,
}

pub(crate) struct StaticElf {
    segments: Vec<Segment>,
    // Packed PT_LOAD file bytes. Zero-fill is synthesized when mapping/fetching.
    bytes: Vec<u8>,
    pub(crate) entry: u64,
    phdr: Option<u64>,
    phnum: u16,
    pub(crate) heap_base: u64,
}

// All callers pass a checked, complete ELF or program header.
fn u16_at(header: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(header[at..at + 2].try_into().unwrap())
}

fn u32_at(header: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(header[at..at + 4].try_into().unwrap())
}

fn u64_at(header: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(header[at..at + 8].try_into().unwrap())
}

impl StaticElf {
    pub(crate) fn parse(file: &[u8]) -> Result<Self, ElfLoadError> {
        let header = file.get(..64).ok_or(Invalid("truncated ELF header"))?;
        if &header[..4] != b"\x7fELF" {
            return Err(Invalid("magic"));
        }
        if header[4..7] != [2, 1, 1] || !matches!(header[7], 0 | 3) || header[8] != 0 {
            return Err(Unsupported("requires little-endian ELF64 for Linux"));
        }
        if u16_at(header, 16) != 2 || u16_at(header, 18) != 62 {
            return Err(Unsupported("requires x86-64 ET_EXEC"));
        }
        if u32_at(header, 20) != 1 || u16_at(header, 52) != 64 || u16_at(header, 54) != 56 {
            return Err(Invalid("header version or size"));
        }
        let entry = u64_at(header, 24);
        let phoff = usize::try_from(u64_at(header, 32)).map_err(|_| Invalid("e_phoff"))?;
        let phnum = u16_at(header, 56);
        // Linux caps the program-header table at 64 KiB.
        if phoff < 64 || phnum == 0 || usize::from(phnum) * 56 > 65536 {
            return Err(Invalid("program header table"));
        }
        let phend = phoff
            .checked_add(usize::from(phnum) * 56)
            .ok_or(Invalid("e_phoff"))?;
        let headers = file
            .get(phoff..phend)
            .ok_or(Invalid("truncated program headers"))?;
        let mut segments = Vec::new();
        let mut phdr = None;
        for ph in headers.chunks(56) {
            let flags = u32_at(ph, 4);
            match u32_at(ph, 0) {
                1 => {} // PT_LOAD
                2 | 3 => return Err(Unsupported("dynamic linking or interpreter")),
                7 => return Err(Unsupported("static TLS initialization")),
                0x6474_e551 if flags & 1 != 0 => return Err(Unsupported("executable stack")),
                _ => continue,
            }
            let offset = u64_at(ph, 8);
            let vaddr = u64_at(ph, 16);
            let filesz = u64_at(ph, 32);
            let memsz = u64_at(ph, 40);
            let align = u64_at(ph, 48);
            if filesz > memsz {
                return Err(Invalid("p_filesz exceeds p_memsz"));
            }
            if memsz == 0 {
                continue;
            }
            if (align > 1 && (!align.is_power_of_two() || vaddr % align != offset % align))
                || (vaddr & 0xfff) != (offset & 0xfff)
            {
                return Err(Invalid("unaligned PT_LOAD"));
            }
            let end = vaddr
                .checked_add(memsz)
                .ok_or(Invalid("PT_LOAD address overflow"))?;
            let data = if filesz == 0 {
                // A pure BSS segment consumes no file bytes, including when
                // p_offset points beyond the last byte on disk.
                0..0
            } else {
                let file_end = offset
                    .checked_add(filesz)
                    .ok_or(Invalid("PT_LOAD file overflow"))?;
                let start = usize::try_from(offset).map_err(|_| Invalid("PT_LOAD offset"))?;
                let finish = usize::try_from(file_end).map_err(|_| Invalid("PT_LOAD file end"))?;
                if finish > file.len() {
                    return Err(Invalid("truncated segment"));
                }
                start..finish
            };
            let protection = match flags {
                4 => Protection::ReadOnly,
                5 => Protection::ReadExecute,
                6 => Protection::ReadWrite,
                _ => return Err(Unsupported("PT_LOAD permissions")),
            };
            if data.start <= phoff && data.end >= phend {
                phdr = Some(vaddr + (phoff - data.start) as u64);
            }
            segments.push(Segment {
                address: vaddr..end,
                data,
                protection,
            });
        }

        segments.sort_by_key(|seg| seg.address.start);
        let first = segments.first().ok_or(Invalid("no loadable segments"))?;
        let last = segments.last().unwrap();
        let heap_base = last
            .address
            .end
            .checked_next_multiple_of(4096)
            .ok_or(Invalid("program break overflow"))?;
        // The current arena spans the whole image; apply the same 1 GiB budget
        // as the PE loader before allocating segment storage.
        if last.address.end - first.address.start > 1 << 30 {
            return Err(Unsupported("image exceeds 1 GiB"));
        }
        if segments
            .windows(2)
            .any(|pair| pair[0].address.end > pair[1].address.start)
        {
            return Err(Invalid("overlapping PT_LOAD segments"));
        }
        if !segments
            .iter()
            .any(|seg| seg.protection.is_executable() && seg.address.contains(&entry))
        {
            return Err(Invalid("entry outside executable PT_LOAD"));
        }
        let mut bytes = Vec::new();
        for segment in &mut segments {
            let start = bytes.len();
            bytes.extend_from_slice(&file[segment.data.clone()]);
            segment.data = start..bytes.len();
        }
        Ok(Self {
            segments,
            bytes,
            entry,
            phdr,
            phnum,
            heap_base,
        })
    }

    pub(crate) fn base(&self) -> u64 {
        self.segments[0].address.start
    }

    pub(crate) fn auxv(&self) -> Vec<(u64, u64)> {
        let mut auxv = vec![(6, 4096), (9, self.entry)]; // AT_PAGESZ, AT_ENTRY
        if let Some(phdr) = self.phdr {
            auxv.extend([(3, phdr), (4, 56), (5, u64::from(self.phnum))]);
        }
        auxv
    }

    pub(crate) fn fetch(&self, pc: u64) -> Option<Vec<u8>> {
        let index = self
            .segments
            .partition_point(|seg| seg.address.start <= pc)
            .checked_sub(1)?;
        let mut window = Vec::new();
        let mut next_pc = pc;
        for seg in &self.segments[index..] {
            if !seg.address.contains(&next_pc) || !seg.protection.is_executable() {
                break;
            }
            let offset = usize::try_from(next_pc - seg.address.start).ok()?;
            let remaining = usize::try_from(seg.address.end - next_pc).ok()?;
            let count = remaining.min(super::FETCH_WINDOW - window.len());
            let end = window.len() + count;
            if let Some(data) = self.bytes[seg.data.clone()].get(offset..) {
                window.extend_from_slice(&data[..count.min(data.len())]);
            }
            window.resize(end, 0);
            if window.len() == super::FETCH_WINDOW {
                break;
            }
            next_pc = seg.address.end;
        }
        (!window.is_empty()).then_some(window)
    }

    pub(crate) fn layout(&self, stack_base: u64) -> Result<GuestLayout, AddressSpaceError> {
        let mut space = AddressSpace::new();
        for seg in &self.segments {
            space.map(
                seg.address.start,
                seg.address.end - seg.address.start,
                seg.protection,
                "PT_LOAD",
            )?;
        }
        let stack = map_default_stack(&mut space, stack_base)?;
        Ok(GuestLayout {
            space,
            image_base: self.base(),
            stack,
        })
    }

    pub(crate) fn populate(
        &self,
        space: &mut BackedAddressSpace,
        stack_base: u64,
    ) -> Result<(), AddressSpaceError> {
        for seg in &self.segments {
            let mut bytes = self.bytes[seg.data.clone()].to_vec();
            let size = usize::try_from(seg.address.end - seg.address.start)
                .expect("validated image fits in 1 GiB");
            bytes.resize(size, 0);
            space.map_with_bytes(seg.address.start, &bytes, seg.protection)?;
        }
        space.map(stack_base, DEFAULT_STACK_SIZE, Protection::ReadWrite)
    }
}
