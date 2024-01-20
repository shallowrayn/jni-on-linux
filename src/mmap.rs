#[cfg(target_os = "linux")]
pub use linux::*;

fn align_down(addr: u64, page_size: u64) -> u64 {
    addr & !(page_size - 1)
}

fn align_up(addr: u64, page_size: u64) -> u64 {
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
        base: *mut c_void,
        size: usize,
    }

    struct LoadCommand {
        pub map_start: u64,  // Virtual address the mapping starts at (aligned)
        pub map_end: u64,    // Virtual address the mapping ends at (aligned)
        pub data_end: u64,   // Virtual address the data from the file ends at (p_vaddr + filesz)
        pub alloc_end: u64,  // Virtual address the data plus extra space ends at (p_vaddr + memsz)
        pub map_offset: u64, // Offset within the file that the mapping starts at (aligned)
        pub map_align: u64,  // Alignment of mapping start and end
        pub prot: ProtFlags,
    }

    impl MemoryMapping {
        pub fn new(file: File, program_headers: &[ProgramHeader]) -> Result<Self, String> {
            // Get the system page size. Memory mappings must lie on page boundaies and be a multiple of the page size
            let page_size = sysconf(SysconfVar::PAGE_SIZE).map_err(|e| e.to_string())?;
            let Some(page_size) = page_size else {
                return Err("Page size cannot be empty".to_string());
            };
            let page_size = page_size as u64;

            let mut load_commands = vec![];
            let mut load_alignment = 0;
            for program_header in program_headers.iter() {
                if program_header.p_type == PT_LOAD {
                    let mut cmd = LoadCommand {
                        map_start: align_down(program_header.p_vaddr, page_size),
                        map_end: align_up(program_header.p_vaddr + program_header.p_memsz, page_size),
                        data_end: program_header.p_vaddr + program_header.p_filesz,
                        alloc_end: program_header.p_vaddr + program_header.p_memsz,
                        map_align: 0,
                        map_offset: align_down(program_header.p_offset, page_size),
                        prot: ProtFlags::PROT_NONE,
                    };
                    load_alignment = std::cmp::max(load_alignment, program_header.p_align);
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

            let mapping_size = (load_commands[load_commands.len() - 1].alloc_end - load_commands[0].map_start) as usize;

            // Reserve enough pages to contain all the mapped program headers. This will be divided later
            let mapping_base = match unsafe {
                mmap::<File>(
                    None,
                    NonZeroUsize::new_unchecked(mapping_size),
                    ProtFlags::PROT_NONE,
                    MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS,
                    None,
                    0,
                )
            } {
                Ok(base) => base,
                Err(errno) => return Err(errno.to_string()),
            };
            // TODO: Obfuscation techniques may rely on gaps between mapped areas. Will PROT_NONE cause a segfault?

            // We now have a block of memory large enough to contain the mapped file, so lets begin loading it. Each
            // segment may have different permissions and permissions are granular to each page, however two segments
            // could share the same page, so we need to make sure not to favour one segment's permissions over the
            // other. This does mean if the end of a read+execute segment is in the same page as the start of a write
            // segment the read+execute segment could modify it's own code in that page, maybe a fun project?
            // Anyway to implement this we loop through each load command and check if it begins in the same page as
            // any previous commands, if so we combine the permissions for just those pages.

            // This should always be zero. Subtract it just in case
            let virtual_mapping_base = load_commands[0].map_start;
            for (mut i, load_command) in load_commands.iter().enumerate() {
                let aligned_start_addr = mapping_base as u64 + load_command.map_start - virtual_mapping_base;
                let aligned_segment_size = load_command.map_end - load_command.map_start;
                let prot = load_command.prot;
                let flags = MapFlags::MAP_PRIVATE | MapFlags::MAP_FIXED;
                let aligned_offset = load_command.map_offset;
                // TODO: What if load_alignment > page_size?
                if let Err(errno) = unsafe {
                    mmap(
                        Some(NonZeroUsize::new_unchecked(aligned_start_addr as usize)),
                        NonZeroUsize::new_unchecked(aligned_segment_size as usize),
                        prot,
                        flags,
                        Some(file.as_fd()),
                        aligned_offset as i64,
                    )
                } {
                    let _ = unsafe { munmap(mapping_base, mapping_size) };
                    return Err(errno.to_string());
                };

                // Check for space after file data
                if load_command.data_end < load_command.alloc_end {
                    // Allow us to write zeroes
                    if prot & ProtFlags::PROT_WRITE != ProtFlags::PROT_WRITE {
                        if let Err(errno) = unsafe {
                            mprotect(
                                aligned_start_addr as *mut c_void,
                                page_size as usize,
                                prot | ProtFlags::PROT_WRITE,
                            )
                        } {
                            let _ = unsafe { munmap(mapping_base, mapping_size) };
                            return Err(errno.to_string());
                        };
                    }
                    // Zero the extra space
                    let data_end = (mapping_base as u64 + load_command.data_end - virtual_mapping_base) as *mut c_void;
                    let extra_space_len = load_command.alloc_end - load_command.data_end;
                    let _ = unsafe { memset(data_end, 0, extra_space_len as usize) };
                    // Remove PROT_WRITE if necessary
                    if prot & ProtFlags::PROT_WRITE != ProtFlags::PROT_WRITE {
                        if let Err(errno) =
                            unsafe { mprotect(aligned_start_addr as *mut c_void, page_size as usize, prot) }
                        {
                            let _ = unsafe { munmap(mapping_base, mapping_size) };
                            return Err(errno.to_string());
                        };
                    }
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
                    if prev_command.map_end > load_command.map_start {
                        have_overlaps = true;
                        overlapped_prot |= prev_command.prot;
                    } else {
                        break;
                    }
                }
                if have_overlaps {
                    if let Err(errno) =
                        unsafe { mprotect(aligned_start_addr as *mut c_void, page_size as usize, overlapped_prot) }
                    {
                        let _ = unsafe { munmap(mapping_base, mapping_size) };
                        return Err(errno.to_string());
                    };
                }
            }

            Ok(Self { base: mapping_base, size: mapping_size })
        }
    }

    impl Drop for MemoryMapping {
        fn drop(&mut self) {
            let _ = unsafe { munmap(self.base, self.size) };
            println!("freed the allocated pages");
        }
    }
}
