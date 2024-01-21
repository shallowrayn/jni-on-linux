#[cfg(target_os = "linux")]
pub use linux::*;

fn align_down(addr: usize, page_size: usize) -> usize {
    addr & !(page_size - 1)
}

fn align_up(addr: usize, page_size: usize) -> usize {
    align_down(addr + page_size - 1, page_size)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{ffi::c_void, fs::File, num::NonZeroUsize, os::fd::AsFd};

    use elf::{
        abi::{PF_R, PF_W, PF_X, PT_LOAD},
        segment::ProgramHeader,
    };
    use nix::{
        libc::memset,
        sys::mman::{mmap, mprotect, munmap, MapFlags, ProtFlags},
        unistd::{sysconf, SysconfVar},
    };

    use super::{align_down, align_up};

    pub struct MemoryMapping {
        pub base: usize,
        pub size: usize,
    }

    struct LoadCommand {
        pub map_start: usize,  // Virtual address the mapping starts at (aligned)
        pub data_end: usize,   // Virtual address the data from the file ends at (p_vaddr + filesz)
        pub alloc_end: usize,  // Virtual address the data plus extra space ends at (p_vaddr + memsz)
        pub map_offset: usize, // Offset within the file that the mapping starts at (aligned)
        pub map_align: usize,  // Alignment of mapping start and end
        pub prot: ProtFlags,
    }

    impl MemoryMapping {
        pub fn new(file: File, program_headers: &[ProgramHeader]) -> Result<Self, String> {
            // Get the system page size. Memory mappings must lie on page boundaies and be a multiple of the page size
            let page_size = sysconf(SysconfVar::PAGE_SIZE).map_err(|e| e.to_string())?;
            let Some(page_size) = page_size else {
                return Err("Page size cannot be empty".to_string());
            };
            let page_size = page_size as usize;

            // Get load commands from program headers
            let mut load_commands = vec![];
            let mut load_alignment = 0;
            for program_header in program_headers.iter() {
                if program_header.p_type == PT_LOAD {
                    let mut cmd = LoadCommand {
                        map_start: align_down(program_header.p_vaddr as usize, page_size),
                        data_end: (program_header.p_vaddr + program_header.p_filesz) as usize,
                        alloc_end: (program_header.p_vaddr + program_header.p_memsz) as usize,
                        map_align: 0,
                        map_offset: align_down(program_header.p_offset as usize, page_size),
                        prot: ProtFlags::PROT_NONE,
                    };
                    load_alignment = std::cmp::max(load_alignment, program_header.p_align as usize);
                    if program_header.p_flags & PF_R == PF_R {
                        cmd.prot |= ProtFlags::PROT_READ;
                    }
                    if program_header.p_flags & PF_W == PF_W {
                        cmd.prot |= ProtFlags::PROT_WRITE;
                    }
                    if program_header.p_flags & PF_X == PF_X {
                        cmd.prot |= ProtFlags::PROT_EXEC;
                    }
                    load_commands.push(cmd);
                }
            }
            // TODO: How to handle alignment larger than page size?
            assert!(
                load_alignment <= page_size,
                "Alignment {load_alignment} larger than page size {page_size}, please open an issue on GitHub"
            );
            for load_command in load_commands.iter_mut() {
                load_command.map_align = load_alignment;
            }

            // This should always be zero. Subtract it just in case
            let virtual_mapping_base = load_commands[0].map_start;
            let mapping_size =
                align_up(load_commands[load_commands.len() - 1].alloc_end - virtual_mapping_base, page_size);

            // Reserve enough pages to contain all the mapped program headers. This will be divided later
            let mapping_base = match unsafe {
                mmap::<File>(
                    None,
                    NonZeroUsize::new_unchecked(mapping_size),
                    ProtFlags::PROT_NONE, /* TODO: Obfuscation techniques may rely on gaps between mapped areas. Will PROT_NONE cause a segfault? */
                    MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS,
                    None,
                    0,
                )
            } {
                Ok(base) => base as usize,
                Err(errno) => return Err(errno.to_string()),
            };

            // We now have a block of memory large enough to contain the mapped file, so lets begin loading it. Each
            // segment may have different permissions and permissions are granular to each page, however two segments
            // could share the same page, so we need to make sure not to favour one segment's permissions over the
            // other. This does mean if the end of a read+execute segment is in the same page as the start of a write
            // segment the read+execute segment could modify it's own code in that page, maybe a fun project?
            // Anyway to implement this we loop through each load command and check if it begins in the same page as
            // any previous commands, if so we combine the permissions for just those pages. We also need to take care
            // of empty space, if the segment's memory size (memsz) is larger than the size on disk (filesz) the extra
            // space should be initialized to zeroes. Mapping past the end of the file would cause a SIGBUS, so we use
            // an anonymous mapping. If the end of the segment doesn't lie on a page boundary the remaining space in
            // that final page should also be initialized to zeroes, for non-writable segments this requires granting
            // write permissions temporarily

            for (mut i, load_command) in load_commands.iter().enumerate() {
                // Map the data from the file
                let aligned_data_addr = mapping_base + load_command.map_start - virtual_mapping_base;
                let aligned_data_size = align_up(load_command.data_end - load_command.map_start, page_size);
                let prot = load_command.prot;
                let aligned_data_offset = load_command.map_offset;
                // TODO: What if load_alignment > page_size?
                if let Err(errno) = unsafe {
                    mmap(
                        Some(NonZeroUsize::new_unchecked(aligned_data_addr)),
                        NonZeroUsize::new_unchecked(aligned_data_size),
                        prot,
                        MapFlags::MAP_PRIVATE | MapFlags::MAP_FIXED,
                        Some(file.as_fd()),
                        aligned_data_offset as i64,
                    )
                } {
                    let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
                    return Err(errno.to_string());
                };

                // Grant us write access if needed
                // NOTE: Changing the permissions of one page causes it to become a second mapping
                if prot & ProtFlags::PROT_WRITE != ProtFlags::PROT_WRITE {
                    let last_data_page_addr = aligned_data_addr + aligned_data_size - page_size;
                    if let Err(errno) =
                        unsafe { mprotect(last_data_page_addr as *mut c_void, page_size, prot | ProtFlags::PROT_WRITE) }
                    {
                        let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
                        return Err(errno.to_string());
                    };
                }

                // Zero the end of the last data page
                let data_end_addr = mapping_base + load_command.data_end - virtual_mapping_base;
                let data_space_size = align_up(data_end_addr, page_size) - data_end_addr;
                let _ = unsafe { memset(data_end_addr as *mut c_void, 0, data_space_size) };

                // Restore the permissions if needed
                if prot & ProtFlags::PROT_WRITE != ProtFlags::PROT_WRITE {
                    let last_data_page_addr = aligned_data_addr + aligned_data_size - page_size;
                    if let Err(errno) = unsafe { mprotect(last_data_page_addr as *mut c_void, page_size, prot) } {
                        let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
                        return Err(errno.to_string());
                    };
                }

                // If the segment needs extra pages, allocate them. Anonymous mappings are zeroed by default
                if align_up(load_command.data_end, page_size) < load_command.alloc_end {
                    let alloc_end_addr = mapping_base + load_command.alloc_end - virtual_mapping_base;
                    let aligned_alloc_start_addr = align_up(data_end_addr, page_size); // The page after the data
                    let aligned_alloc_end_addr = align_up(alloc_end_addr, page_size);
                    let aligned_alloc_size = aligned_alloc_end_addr - aligned_alloc_start_addr;
                    if let Err(errno) = unsafe {
                        mmap::<File>(
                            Some(NonZeroUsize::new_unchecked(aligned_alloc_start_addr)),
                            NonZeroUsize::new_unchecked(aligned_alloc_size),
                            prot,
                            MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS | MapFlags::MAP_FIXED,
                            None,
                            0,
                        )
                    } {
                        let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
                        return Err(errno.to_string());
                    };
                }

                // Check for overlapping pages and handle accordingly
                let mut have_overlaps = false;
                let mut overlapped_prot = prot;
                loop {
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                    let prev_command = &load_commands[i];
                    let aligned_prev_end = align_up(prev_command.alloc_end, page_size);
                    if aligned_prev_end > load_command.map_start {
                        have_overlaps = true;
                        overlapped_prot |= prev_command.prot;
                    } else {
                        break;
                    }
                }
                if have_overlaps {
                    if let Err(errno) =
                        unsafe { mprotect(aligned_data_addr as *mut c_void, page_size, overlapped_prot) }
                    {
                        let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
                        return Err(errno.to_string());
                    };
                }
            }

            Ok(Self { base: mapping_base, size: mapping_size })
        }
    }

    impl Drop for MemoryMapping {
        fn drop(&mut self) {
            let _ = unsafe { munmap(self.base as *mut c_void, self.size) };
        }
    }
}
