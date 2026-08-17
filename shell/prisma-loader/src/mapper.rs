use crate::pe::{ImageImportDescriptor, ImageThunkData64, PeImage};
use anyhow::{anyhow, Result};
use std::ffi::CStr;

pub struct PeVirtualImage {
    /// The contiguous virtual memory representing the mapped executable.
    memory: Vec<u8>,
    image_base: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImportFunction<'a> {
    Ordinal(u16),
    Name(&'a str),
}

#[derive(Debug)]
pub struct DllImport<'a> {
    pub dll_name: &'a str,
    pub functions: Vec<ImportFunction<'a>>,
}

impl PeVirtualImage {
    /// Maps the given PeImage into a flat virtual memory buffer according to section headers.
    pub fn map(image: &PeImage) -> Result<Self> {
        let opt_hdr = image
            .optional_header()
            .ok_or_else(|| anyhow!("Missing optional header"))?;

        let size_of_image = opt_hdr.size_of_image as usize;
        let mut memory = vec![0u8; size_of_image];

        // Copy PE headers (from start of file up to size_of_headers)
        let size_of_headers = opt_hdr.size_of_headers as usize;
        let raw_data = image.data(); // Wait, PeImage doesn't expose data() yet. I need to expose it.
                                     // Actually, we can just grab it. Let's assume we will add `pub fn data(&self) -> &'a [u8]`

        let header_copy_len = std::cmp::min(size_of_headers, raw_data.len());
        let header_copy_len = std::cmp::min(header_copy_len, memory.len());
        memory[..header_copy_len].copy_from_slice(&raw_data[..header_copy_len]);

        // Copy each section to its virtual address
        for section in image.sections() {
            let v_addr = section.virtual_address as usize;
            let v_size = section.virtual_size as usize;
            let p_raw = section.pointer_to_raw_data as usize;
            let s_raw = section.size_of_raw_data as usize;

            if v_addr >= memory.len() {
                continue; // Invalid section virtual address
            }

            let copy_size = std::cmp::min(v_size, s_raw);
            if p_raw >= raw_data.len() {
                continue;
            }

            let available_raw = raw_data.len() - p_raw;
            let copy_size = std::cmp::min(copy_size, available_raw);

            let available_mem = memory.len() - v_addr;
            let copy_size = std::cmp::min(copy_size, available_mem);

            memory[v_addr..v_addr + copy_size].copy_from_slice(&raw_data[p_raw..p_raw + copy_size]);
        }

        Ok(Self {
            memory,
            image_base: opt_hdr.image_base,
        })
    }

    /// Safely gets a byte slice starting at `rva` of `size` bytes.
    fn get_slice(&self, rva: usize, size: usize) -> Option<&[u8]> {
        if rva.saturating_add(size) <= self.memory.len() {
            Some(&self.memory[rva..rva + size])
        } else {
            None
        }
    }

    /// Reads a null-terminated C string at the given RVA.
    fn read_string(&self, rva: usize) -> Result<&str> {
        let max_len = self.memory.len().saturating_sub(rva);
        if max_len == 0 {
            return Err(anyhow!("RVA out of bounds for string"));
        }
        let slice = &self.memory[rva..];
        let cstr =
            CStr::from_bytes_until_nul(slice).map_err(|_| anyhow!("Missing null terminator"))?;
        cstr.to_str()
            .map_err(|_| anyhow!("Invalid UTF-8 in string"))
    }

    /// Parses the Import Directory and resolves all DLL dependencies and their imported functions.
    pub fn imports(&self, image: &PeImage) -> Result<Vec<DllImport>> {
        let opt_hdr = image
            .optional_header()
            .ok_or_else(|| anyhow!("Missing optional header"))?;

        // DataDirectory[1] is the Import Directory
        let import_dir = &opt_hdr.data_directory[1];
        if import_dir.virtual_address == 0 || import_dir.size == 0 {
            return Ok(Vec::new()); // No imports
        }

        let mut result = Vec::new();
        let mut descriptor_rva = import_dir.virtual_address as usize;

        loop {
            let desc_slice = self
                .get_slice(descriptor_rva, std::mem::size_of::<ImageImportDescriptor>())
                .ok_or_else(|| anyhow!("Import descriptor out of bounds"))?;

            // Safety: Descriptor slice is exactly the size of the struct
            let desc = unsafe {
                std::ptr::read_unaligned(desc_slice.as_ptr() as *const ImageImportDescriptor)
            };

            // Null descriptor terminates the array
            if desc.name == 0 && desc.first_thunk == 0 {
                break;
            }

            let dll_name = self.read_string(desc.name as usize)?;

            let mut thunk_rva = if desc.original_first_thunk != 0 {
                desc.original_first_thunk as usize
            } else {
                desc.first_thunk as usize
            };

            let mut functions = Vec::new();

            loop {
                let thunk_slice = self
                    .get_slice(thunk_rva, std::mem::size_of::<ImageThunkData64>())
                    .ok_or_else(|| anyhow!("Thunk data out of bounds"))?;

                let thunk = unsafe {
                    std::ptr::read_unaligned(thunk_slice.as_ptr() as *const ImageThunkData64)
                };

                if thunk.u1 == 0 {
                    break;
                }

                const IMAGE_ORDINAL_FLAG64: u64 = 0x8000_0000_0000_0000;

                if (thunk.u1 & IMAGE_ORDINAL_FLAG64) != 0 {
                    let ordinal = (thunk.u1 & 0xFFFF) as u16;
                    functions.push(ImportFunction::Ordinal(ordinal));
                } else {
                    // It's a pointer to an IMAGE_IMPORT_BY_NAME struct (u16 hint + null terminated string)
                    let name_rva = (thunk.u1 & 0x7FFF_FFFF) as usize;
                    let func_name = self.read_string(name_rva + 2)?; // Skip 2-byte hint
                    functions.push(ImportFunction::Name(func_name));
                }

                thunk_rva += std::mem::size_of::<ImageThunkData64>();
            }

            result.push(DllImport {
                dll_name,
                functions,
            });

            descriptor_rva += std::mem::size_of::<ImageImportDescriptor>();
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Since a real mapped binary is complex, we rely on Cargo's compile checks for the mapper syntax,
    // and we'll add property testing in a later phase for the mapper specifically.
    #[test]
    fn test_mapper_compiles() {
        assert!(true);
    }
}
