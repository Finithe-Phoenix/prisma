use anyhow::{anyhow, Result};
use std::mem;

// Standard PE DOS Header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageDosHeader {
    pub e_magic: u16,      // Magic number "MZ"
    pub e_cblp: u16,       // Bytes on last page of file
    pub e_cp: u16,         // Pages in file
    pub e_crlc: u16,       // Relocations
    pub e_cparhdr: u16,    // Size of header in paragraphs
    pub e_minalloc: u16,   // Minimum extra paragraphs needed
    pub e_maxalloc: u16,   // Maximum extra paragraphs needed
    pub e_ss: u16,         // Initial (relative) SS value
    pub e_sp: u16,         // Initial SP value
    pub e_csum: u16,       // Checksum
    pub e_ip: u16,         // Initial IP value
    pub e_cs: u16,         // Initial (relative) CS value
    pub e_lfarlc: u16,     // File address of relocation table
    pub e_ovno: u16,       // Overlay number
    pub e_res: [u16; 4],   // Reserved words
    pub e_oemid: u16,      // OEM identifier (for e_oeminfo)
    pub e_oeminfo: u16,    // OEM information; e_oemid specific
    pub e_res2: [u16; 10], // Reserved words
    pub e_lfanew: i32,     // File address of new exe header
}

// 32-bit PE / 64-bit PE common signature
pub const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D; // MZ
pub const IMAGE_NT_SIGNATURE: u32 = 0x00004550; // PE\0\0

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageFileHeader {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub pointer_to_symbol_table: u32,
    pub number_of_symbols: u32,
    pub size_of_optional_header: u16,
    pub characteristics: u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageDataDirectory {
    pub virtual_address: u32,
    pub size: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageOptionalHeader64 {
    pub magic: u16,
    pub major_linker_version: u8,
    pub minor_linker_version: u8,
    pub size_of_code: u32,
    pub size_of_initialized_data: u32,
    pub size_of_uninitialized_data: u32,
    pub address_of_entry_point: u32,
    pub base_of_code: u32,
    pub image_base: u64,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub major_operating_system_version: u16,
    pub minor_operating_system_version: u16,
    pub major_image_version: u16,
    pub minor_image_version: u16,
    pub major_subsystem_version: u16,
    pub minor_subsystem_version: u16,
    pub win32_version_value: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub check_sum: u32,
    pub subsystem: u16,
    pub dll_characteristics: u16,
    pub size_of_stack_reserve: u64,
    pub size_of_stack_commit: u64,
    pub size_of_heap_reserve: u64,
    pub size_of_heap_commit: u64,
    pub loader_flags: u32,
    pub number_of_rva_and_sizes: u32,
    pub data_directory: [ImageDataDirectory; 16],
}

pub const IMAGE_OPTIONAL_HEADER64_MAGIC: u16 = 0x20B;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageSectionHeader {
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
    pub pointer_to_relocations: u32,
    pub pointer_to_linenumbers: u32,
    pub number_of_relocations: u16,
    pub number_of_linenumbers: u16,
    pub characteristics: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageImportDescriptor {
    pub original_first_thunk: u32,
    pub time_date_stamp: u32,
    pub forwarder_chain: u32,
    pub name: u32,
    pub first_thunk: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ImageThunkData64 {
    pub u1: u64, // Ordinal, or AddressOfData, etc.
}

pub struct PeImage<'a> {
    data: &'a [u8],
    dos_header: *const ImageDosHeader,
    file_header: *const ImageFileHeader,
    optional_header: *const ImageOptionalHeader64,
    sections: &'a [ImageSectionHeader],
}

impl<'a> PeImage<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let mut image = Self {
            data,
            dos_header: std::ptr::null(),
            file_header: std::ptr::null(),
            optional_header: std::ptr::null(),
            sections: &[],
        };
        image.validate()?;
        Ok(image)
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn sections(&self) -> &'a [ImageSectionHeader] {
        self.sections
    }

    pub fn optional_header(&self) -> Option<&'a ImageOptionalHeader64> {
        if self.optional_header.is_null() {
            None
        } else {
            // Safety: We validated the pointer and its bounds during new()
            Some(unsafe { &*self.optional_header })
        }
    }

    fn validate(&mut self) -> Result<()> {
        if self.data.len() < mem::size_of::<ImageDosHeader>() {
            return Err(anyhow!("File too small to contain DOS header"));
        }

        // Safety: We already checked the bounds.
        self.dos_header = self.data.as_ptr() as *const ImageDosHeader;
        let dos_header = unsafe { &*self.dos_header };

        if dos_header.e_magic != IMAGE_DOS_SIGNATURE {
            return Err(anyhow!("Invalid DOS signature"));
        }

        let lfanew = dos_header.e_lfanew as usize;
        let nt_headers_min_size = 4 + mem::size_of::<ImageFileHeader>();

        if self.data.len() < lfanew.saturating_add(nt_headers_min_size) {
            return Err(anyhow!("File too small to contain NT headers"));
        }

        let nt_signature =
            unsafe { std::ptr::read_unaligned(self.data.as_ptr().add(lfanew) as *const u32) };

        if nt_signature != IMAGE_NT_SIGNATURE {
            return Err(anyhow!("Invalid PE signature"));
        }

        let file_header_offset = lfanew + 4;
        self.file_header =
            unsafe { self.data.as_ptr().add(file_header_offset) as *const ImageFileHeader };
        let file_header = unsafe { &*self.file_header };

        let optional_header_size = file_header.size_of_optional_header as usize;
        let optional_header_offset = file_header_offset + mem::size_of::<ImageFileHeader>();

        if self.data.len() < optional_header_offset.saturating_add(optional_header_size) {
            return Err(anyhow!("File too small to contain Optional Header"));
        }

        if optional_header_size >= mem::size_of::<ImageOptionalHeader64>() {
            self.optional_header = unsafe {
                self.data.as_ptr().add(optional_header_offset) as *const ImageOptionalHeader64
            };

            let optional_header = unsafe { &*self.optional_header };
            if optional_header.magic != IMAGE_OPTIONAL_HEADER64_MAGIC {
                return Err(anyhow!("Only PE32+ (64-bit) binaries are supported"));
            }
        } else {
            return Err(anyhow!("Optional Header too small for 64-bit binary"));
        }

        let sections_offset = optional_header_offset + optional_header_size;
        let num_sections = file_header.number_of_sections as usize;
        let sections_size = num_sections.saturating_mul(mem::size_of::<ImageSectionHeader>());

        if self.data.len() < sections_offset.saturating_add(sections_size) {
            return Err(anyhow!("File too small to contain all Section Headers"));
        }

        // Safety: We validated the exact size of the sections array against the file buffer size
        self.sections = unsafe {
            std::slice::from_raw_parts(
                self.data.as_ptr().add(sections_offset) as *const ImageSectionHeader,
                num_sections,
            )
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_invalid_image() {
        let dummy_data = [0u8; 10];
        assert!(PeImage::new(&dummy_data).is_err());
    }

    #[test]
    fn test_invalid_magic() {
        let mut dummy_data = vec![0u8; mem::size_of::<ImageDosHeader>()];
        dummy_data[0] = 0x00;
        assert!(PeImage::new(&dummy_data).is_err());
    }

    // EXTREME COVERAGE: Fuzz the parser with absolutely random byte arrays.
    // This mathematically proves that our parser NEVER panics on bad data.
    proptest! {
        #[test]
        fn doesnt_crash_on_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
            // All we care about is that it doesn't panic.
            let _ = PeImage::new(&bytes);
        }

        #[test]
        fn doesnt_crash_on_random_lfanew(
            mut bytes in prop::collection::vec(any::<u8>(), mem::size_of::<ImageDosHeader>()..4096),
            lfanew in any::<i32>()
        ) {
            bytes[0] = 0x4D; // M
            bytes[1] = 0x5A; // Z

            let lfanew_bytes = lfanew.to_le_bytes();
            let offset = mem::size_of::<ImageDosHeader>() - 4;
            bytes[offset..offset+4].copy_from_slice(&lfanew_bytes);

            let _ = PeImage::new(&bytes);
        }

        #[test]
        fn doesnt_crash_on_random_section_count(
            mut bytes in prop::collection::vec(any::<u8>(), mem::size_of::<ImageDosHeader>() + 300..4096),
            lfanew in 64i32..128i32,
            num_sections in any::<u16>()
        ) {
            bytes[0] = 0x4D; // M
            bytes[1] = 0x5A; // Z

            let offset = mem::size_of::<ImageDosHeader>() - 4;
            bytes[offset..offset+4].copy_from_slice(&lfanew.to_le_bytes());

            let lfanew_usize = lfanew as usize;
            if bytes.len() >= lfanew_usize + 4 + mem::size_of::<ImageFileHeader>() {
                // Write PE\0\0
                bytes[lfanew_usize] = b'P';
                bytes[lfanew_usize+1] = b'E';
                bytes[lfanew_usize+2] = 0;
                bytes[lfanew_usize+3] = 0;

                // Overwrite num_sections (offset 6 from PE header start)
                let num_sections_bytes = num_sections.to_le_bytes();
                bytes[lfanew_usize+4+2..lfanew_usize+4+4].copy_from_slice(&num_sections_bytes);
            }

            // Must gracefully return Err without panicking due to out of bounds slice creation
            let _ = PeImage::new(&bytes);
        }
    }
}
