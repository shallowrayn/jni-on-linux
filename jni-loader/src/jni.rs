use std::{
    collections::HashMap,
    fmt::Debug,
    fs::{self, File},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use elf::{
    abi::{DT_NEEDED, DT_RUNPATH, ET_DYN, PT_LOAD},
    endian::AnyEndian,
    hash::{GnuHashTable, SysVHashTable},
    relocation::{Rel, Rela},
    symbol::Symbol,
    ElfStream,
};
use log::{debug, error, info, trace};
use thiserror::Error;

#[cfg(feature = "inline-asm")]
use super::plt;
use super::{locate, mmap::MemoryMapping, plt::PltData};

pub struct JNI {
    path: PathBuf,
    name: String,
    elf_file: ElfStream<AnyEndian, File>,
    mapping: MemoryMapping,
    base_virtual_address: usize, // Lowest PT_LOAD virtual address
    dependencies: HashMap<String, Option<Arc<Mutex<Box<JNI>>>>>,
    loaded_dependencies: bool,
    have_been_initialized: bool,
    symbol_overrides: HashMap<String, Option<usize>>,
    looking_for_symbol: bool,
    plt_data: Option<PltData>,
}

const UNDEFINED_SYMBOL_VALUE: usize = 0xBABECAFE;
const STN_UNDEF: u64 = 0; // Undefined symbol

impl JNI {
    pub fn new(path: PathBuf) -> Result<Box<Self>, Error> {
        if !path.exists() {
            return Err(Error::FileNotFound);
        }
        let Ok(file) = File::open(path.clone()) else {
            return Err(Error::FailedToOpen);
        };
        let Ok(mapping_file) = File::open(path.clone()) else {
            return Err(Error::FailedToOpen);
        };
        let Ok(elf_file) = ElfStream::open_stream(file) else {
            return Err(Error::FailedToOpen);
        };
        if elf_file.ehdr.e_type != ET_DYN {
            return Err(Error::NotDynamicObject);
        }
        let name = path.clone().file_name().unwrap().to_str().unwrap().to_owned();
        info!(target: &name, "Trying to memory map {:?}", fs::canonicalize(path.clone()).unwrap_or(path.clone()));
        let mapping = match MemoryMapping::new(mapping_file, elf_file.segments()) {
            Ok(mapping) => mapping,
            Err(error) => return Err(Error::MemoryMapFailed(error)),
        };
        let base_virtual_address = elf_file.segments().iter().find(|&s| s.p_type == PT_LOAD).unwrap().p_vaddr as usize;
        let mut jni = Box::new(Self {
            path,
            name,
            elf_file,
            mapping,
            base_virtual_address,
            dependencies: HashMap::new(),
            loaded_dependencies: false,
            have_been_initialized: false,
            symbol_overrides: HashMap::new(),
            looking_for_symbol: false,
            plt_data: None,
        });
        let jni_addr = &mut *jni as *mut JNI;
        jni.plt_data = Some(PltData::new(jni_addr));
        Ok(jni)
    }

    pub fn new_from_name(name: &str) -> Result<Box<Self>, Error> {
        match locate::locate_library(name, None) {
            Some(lib_path) => Self::new(lib_path),
            None => Err(Error::FileNotFound),
        }
    }

    pub fn add_dependency(&mut self, name: &str, lib: Option<Box<JNI>>) {
        self.dependencies.insert(name.to_string(), lib.map(Mutex::new).map(Arc::new));
    }

    pub fn add_shared_dependency(&mut self, name: &str, lib: Option<Arc<Mutex<Box<JNI>>>>) {
        self.dependencies.insert(name.to_string(), lib);
    }

    pub fn load_dependencies(&mut self) -> Result<(), Error> {
        if self.loaded_dependencies {
            return Ok(());
        }
        self.loaded_dependencies = true;

        // Dependencies are stored using DT_NEEDED keys in the .dynamic section. We also need DT_RUNPATH for locating
        let Ok(Some(dynamic_section)) = self.elf_file.dynamic() else {
            return Err(Error::NoDyanmicSection);
        };
        let mut dependency_offsets = Vec::new();
        let mut dt_runpath_offset = None;
        for entry in dynamic_section {
            match entry.d_tag {
                DT_NEEDED => {
                    dependency_offsets.push(entry.d_val() as usize);
                },
                DT_RUNPATH => {
                    dt_runpath_offset = Some(entry.d_val() as usize);
                },
                _ => {},
            }
        }

        // Values from .dynamic are offsets into the .dynstr string table
        let Ok(Some(dynamic_string_table_header)) = self.elf_file.section_header_by_name(".dynstr") else {
            return Err(Error::NoDyanmicSection);
        };
        let dynamic_string_table_header = *dynamic_string_table_header; // End mutable borrow of self.elf_file
        let Ok(dynamic_string_table) = self.elf_file.section_data_as_strtab(&dynamic_string_table_header) else {
            return Err(Error::NoDyanmicSection);
        };
        let dependencies: Vec<String> = dependency_offsets
            .into_iter()
            .flat_map(|offset| dynamic_string_table.get(offset))
            .map(|s| s.to_string())
            .collect();

        // Loop through dependencies, if they haven't been overridden then try to locate and load them
        let parent_dir = self.path.parent().map(PathBuf::from);
        let dt_runpath = dt_runpath_offset.and_then(|offset| dynamic_string_table.get(offset).ok()).map(PathBuf::from);
        for lib_name in dependencies {
            trace!(target: &self.name, "Looking for dependency {lib_name}");
            if let Some(dependency) = self.dependencies.get(&lib_name) {
                debug!(target: &self.name, "Found dependency {lib_name} - {:?}", dependency.as_ref().map(|d| d.lock().unwrap().path.to_owned()));
                continue;
            }
            match locate::locate_library_internal(&lib_name, None, parent_dir.clone(), dt_runpath.clone()) {
                Some(lib_path) => {
                    let dependency = JNI::new(lib_path).ok();
                    debug!(target: &self.name, "Found dependency {lib_name} - {:?}", dependency.as_ref().map(|d| d.path.to_owned()));
                    self.dependencies.insert(lib_name, dependency.map(Mutex::new).map(Arc::new));
                },
                None => {
                    debug!(target: &self.name, "Found dependency {lib_name} - None");
                    self.dependencies.insert(lib_name, None);
                },
            }
        }
        Ok(())
    }

    pub fn override_symbol(&mut self, symbol_name: &str, new_value: Option<*const ()>) {
        trace!(target: &self.name, "Overriding symbol {symbol_name} with {new_value:?}");
        self.symbol_overrides.insert(symbol_name.to_owned(), new_value.map(|v| v as usize));
    }

    pub fn get_symbol(&mut self, symbol_name: &str) -> Option<(*const (), u64)> {
        let mut symbol = self.find_local_symbol_by_name(symbol_name, false);
        if symbol.is_none() {
            symbol = self.find_global_symbol(symbol_name, false);
        }
        symbol.map(|symbol| (self.get_offset(symbol.value as usize) as *const (), symbol.size))
    }

    pub fn get_offset(&self, offset: usize) -> usize {
        self.mapping.base + offset - self.base_virtual_address
    }

    pub fn initialize(&mut self) {
        if self.have_been_initialized {
            return;
        }
        self.have_been_initialized = true;
        debug!(target: &self.name, "Initializing");

        for (_, dependency) in self.dependencies.iter() {
            if let Some(dependency) = dependency {
                // NOTE - Deadlocks
                // The guard at the top of this function prevents this loop being recursively executed on one instance
                // If A depends on B and A and B both depend on C, C's lock will be released before B::initialize()
                let mut dependency = dependency.lock().unwrap();
                debug!(target: &self.name, "Initializing dependency {}", dependency.name);
                dependency.initialize();
            }
        }

        // Apply relocations. There are different types of relocation, with and without an addend. These are stored in
        // different sections (.rel.<set> without and .rela.<set> with). There is also .relr.dyn which is the recently
        // redesigned relative relocations however they aren't fully adopted yet [1]. There are two sets of relocations
        // we are interested in, "dyn" and "plt". "dyn" relocations include the GOT entries and any relocation needed
        // before code can execute. "plt" relocations can be lazily filled in using the PLT callback in .got[2] or can
        // be filled in while loading (RTLD_NOW). If we fill them in while loading we don't need a PLT callback at all
        // meaning no assembly.
        //
        // [1] https://maskray.me/blog/2021-10-31-relative-relocations-and-relr
        let mut relocations = Vec::new();
        if let Ok(Some(&rel_dyn_header)) = self.elf_file.section_header_by_name(".rel.dyn") {
            if let Ok(rel_dyn) = self.elf_file.section_data_as_rels(&rel_dyn_header) {
                let old_len = relocations.len();
                relocations.extend(rel_dyn.map(Relocation::from));
                debug!(target: &self.name, "Added {} relocations from .rel.dyn", relocations.len() - old_len);
            }
        }
        if let Ok(Some(&rela_dyn_header)) = self.elf_file.section_header_by_name(".rela.dyn") {
            if let Ok(rela_dyn) = self.elf_file.section_data_as_relas(&rela_dyn_header) {
                let old_len = relocations.len();
                relocations.extend(rela_dyn.map(Relocation::from));
                debug!(target: &self.name, "Added {} relocations from .rela.dyn", relocations.len() - old_len);
            }
        }
        // Without inline assembly we don't have a PLT trampoline. Resolve all PLT entries now
        #[cfg(not(feature = "inline-asm"))]
        {
            if let Ok(Some(&rel_plt_header)) = self.elf_file.section_header_by_name(".rel.plt") {
                if let Ok(rel_plt) = self.elf_file.section_data_as_rels(&rel_plt_header) {
                    let old_len = relocations.len();
                    relocations.extend(rel_plt.map(Relocation::from));
                    debug!(target: &self.name, "Assembly disabled, added {} relocations from .rel.plt", relocations.len() - old_len);
                }
            }
            if let Ok(Some(&rela_plt_header)) = self.elf_file.section_header_by_name(".rela.plt") {
                if let Ok(rela_plt) = self.elf_file.section_data_as_relas(&rela_plt_header) {
                    let old_len = relocations.len();
                    relocations.extend(rela_plt.map(Relocation::from));
                    debug!(target: &self.name, "Assembly disabled, added {} relocations from .rela.plt", relocations.len() - old_len);
                }
            }
        }
        for relocation in relocations {
            let target_addr = self.get_offset(relocation.offset);
            // Only some relocations need the symbol
            macro_rules! reloc_needs_symbol {
                ($reloc:expr) => {{
                    let symbol = if relocation.symbol != 0 {
                        let Some(local_symbol) = self.find_local_symbol_by_index(relocation.symbol, true) else {
                            continue;
                        };
                        if local_symbol.address.is_some() {
                            Some(local_symbol)
                        } else {
                            let symbol_name = local_symbol.name.expect("Cannot lookup symbol without name");
                            self.find_global_symbol(&symbol_name, true)
                        }
                    } else {
                        None
                    };
                    match symbol {
                        Some(s) => s.address.unwrap_or(self.get_offset(s.value as usize)),
                        None => continue,
                    }
                }};
            }
            #[cfg(target_pointer_width = "64")]
            trace!(target: &self.name, "Processing {relocation:?} at {:#018x}", target_addr);
            #[cfg(not(target_pointer_width = "64"))]
            trace!(target: &self.name, "Processing {relocation:?} at {:#010x}", target_addr);

            #[cfg(target_arch = "x86_64")]
            match relocation.rel_type {
                elf::abi::R_X86_64_64 => {
                    let symbol_addr = reloc_needs_symbol!("R_X86_64_64");
                    unsafe { *(target_addr as *mut u64) = add_addend(symbol_addr, relocation.addend) as u64 };
                },
                elf::abi::R_X86_64_PC32 => {
                    let symbol_addr = reloc_needs_symbol!("R_X86_64_PC32");
                    unsafe {
                        *(target_addr as *mut u32) = (add_addend(symbol_addr, relocation.addend) - target_addr) as u32
                    };
                },
                elf::abi::R_X86_64_GLOB_DAT | elf::abi::R_X86_64_JUMP_SLOT => {
                    let symbol_addr = reloc_needs_symbol!("R_X86_64_GLOB_DAT / R_X86_64_JUMP_SLOT");
                    unsafe { *(target_addr as *mut u64) = symbol_addr as u64 };
                },
                elf::abi::R_X86_64_RELATIVE => {
                    unsafe { *(target_addr as *mut u64) = add_addend(self.mapping.base, relocation.addend) as u64 };
                },
                elf::abi::R_X86_64_NONE | elf::abi::R_X86_64_COPY => {},
                _ => {
                    #[cfg(debug_assertions)]
                    panic!(
                        "Failed to handle relocation type {:#08x} for offset {:#012x}",
                        relocation.rel_type, relocation.offset
                    );
                },
            }
            #[cfg(target_arch = "aarch64")]
            match relocation.rel_type {
                elf::abi::R_AARCH64_GLOB_DAT | elf::abi::R_AARCH64_JUMP_SLOT | elf::abi::R_AARCH64_ABS64 => {
                    let symbol_addr = reloc_needs_symbol!("R_AARCH64_JUMP_SLOT");
                    unsafe { *(target_addr as *mut u64) = add_addend(symbol_addr, relocation.addend) as u64 };
                },
                elf::abi::R_AARCH64_RELATIVE => {
                    unsafe { *(target_addr as *mut u64) = add_addend(self.mapping.base, relocation.addend) as u64 };
                },
                _ => {
                    #[cfg(debug_assertions)]
                    panic!(
                        "Failed to handle relocation type {:#08x} for offset {:#012x}",
                        relocation.rel_type, relocation.offset
                    );
                },
            }
            #[cfg(all(not(target_arch = "x86_64"), not(target_arch = "aarch64")))]
            panic!("Unhandled system architecture")
        }

        // Set up the PLT handler if needed
        #[cfg(feature = "inline-asm")]
        if let Ok(Some(&got_plt_header)) = self.elf_file.section_header_by_name(".got.plt") {
            let got_entry_count = got_plt_header.sh_size as usize / std::mem::size_of::<usize>();
            let got_plt_addr = self.get_offset(got_plt_header.sh_addr as usize);
            let plt_0 = 0xCAFEBABE;
            let plt_1 = self.plt_data.as_ref().unwrap() as *const PltData as usize;
            let plt_2 = plt::trampoline as usize;
            #[cfg(target_pointer_width = "64")]
            {
                debug!(target: &self.name, "Updating {} .got.plt entries at {:#018x}-{:#018x} using base address {:#018x}", got_entry_count, got_plt_addr, got_plt_addr + got_plt_header.sh_size as usize, self.get_offset(0));
                debug!(target: &self.name, ".got.plt[0] {:#010x}", plt_0);
                debug!(target: &self.name, ".got.plt[1] {:#018x}", plt_1);
                debug!(target: &self.name, ".got.plt[2] {:#018x}", plt_2);
            }
            #[cfg(not(target_pointer_width = "64"))]
            {
                debug!(target: &self.name, "Updating {} .got.plt entries at {:#010x}-{:#010x} using base address {:#010x}", got_entry_count, got_plt_addr, got_plt_addr + got_plt_header.sh_size as usize, self.get_offset(0));
                debug!(target: &self.name, ".got.plt[0] {:#010x}", plt_0);
                debug!(target: &self.name, ".got.plt[1] {:#010x}", plt_1);
                debug!(target: &self.name, ".got.plt[2] {:#010x}", plt_2);
            }
            let got_plt_entries =
                unsafe { std::slice::from_raw_parts_mut(got_plt_addr as *mut usize, got_entry_count) };
            got_plt_entries[0] = plt_0;
            got_plt_entries[1] = plt_1;
            got_plt_entries[2] = plt_2;
            for entry in got_plt_entries[3..got_entry_count].iter_mut() {
                *entry = self.get_offset(*entry);
            }
        }

        debug!(target: &self.name, "Initialized");
    }

    // Look for a local symbol using its index
    fn find_local_symbol_by_index(&mut self, index: u32, include_overrides: bool) -> Option<LinkingSymbol> {
        trace!(target: &self.name, "Looking for symbol {index}");
        let (symbol_table, symbol_string_table) = self.elf_file.dynamic_symbol_table().ok()??;
        let symbol = symbol_table.get(index as usize).ok()?;
        let mut symbol_name = None;
        if symbol.st_name != 0 {
            let sym_name = symbol_string_table.get(symbol.st_name as usize).ok()?.to_owned();
            trace!(target: &self.name, r#"Found name "{sym_name}" for index {index}"#);
            if include_overrides {
                if let Some(&overridden_value) = self.symbol_overrides.get(&sym_name) {
                    let address = overridden_value.unwrap_or(UNDEFINED_SYMBOL_VALUE);
                    #[cfg(target_pointer_width = "64")]
                    trace!(target: &self.name, r#"Found override {:#018x} for "{}""#, address, sym_name);
                    #[cfg(not(target_pointer_width = "64"))]
                    trace!(target: &self.name, r#"Found override {:#010x} for "{}""#, address, sym_name);
                    return Some(LinkingSymbol::from_override(&symbol, Some(sym_name), address));
                }
            }
            symbol_name = Some(sym_name);
        }
        Some(LinkingSymbol::from(&symbol, symbol_name, self.mapping.base, self.base_virtual_address))
    }

    // Look for a local symbol using the hash tables
    fn find_local_symbol_by_name(&mut self, symbol_name: &str, include_overrides: bool) -> Option<LinkingSymbol> {
        trace!(target: &self.name, r#"Looking for symbol "{symbol_name}" in hash tables"#);
        // Check .gnu.hash first as it is faster
        if let Ok(Some(&gnu_hash_section_header)) = self.elf_file.section_header_by_name(".gnu.hash") {
            let elf_endianness = self.elf_file.ehdr.endianness;
            let elf_class = self.elf_file.ehdr.class;
            // Only .debug sections can be compressed
            let (gnu_hash_section, _) = self.elf_file.section_data(&gnu_hash_section_header).ok()?;
            let gnu_hash_section = gnu_hash_section.to_vec(); // Can't have two references into self.elf_file so copy the section
            let hash_section = GnuHashTable::new(elf_endianness, elf_class, &gnu_hash_section).ok()?;
            let (symbol_table, symbol_string_table) = self.elf_file.dynamic_symbol_table().ok()??;
            if let Some((_, symbol)) =
                hash_section.find(symbol_name.as_bytes(), &symbol_table, &symbol_string_table).ok()?
            {
                trace!(target: &self.name, r#"Found "{symbol_name}" in .gnu.hash"#);
                if include_overrides {
                    if let Some(&overridden_value) = self.symbol_overrides.get(symbol_name) {
                        let address = overridden_value.unwrap_or(UNDEFINED_SYMBOL_VALUE);
                        #[cfg(target_pointer_width = "64")]
                        trace!(target: &self.name, r#"Found override {:#018x} for "{}""#, address, symbol_name);
                        #[cfg(not(target_pointer_width = "64"))]
                        trace!(target: &self.name, r#"Found override {:#010x} for "{}""#, address, symbol_name);
                        return Some(LinkingSymbol::from_override(&symbol, Some(symbol_name.to_owned()), address));
                    }
                }
                return Some(LinkingSymbol::from(
                    &symbol,
                    Some(symbol_name.to_owned()),
                    self.mapping.base,
                    self.base_virtual_address,
                ));
            }
        }

        // Check .hash
        if let Ok(Some(&sysv_hash_section_header)) = self.elf_file.section_header_by_name(".hash") {
            let elf_endianness = self.elf_file.ehdr.endianness;
            let elf_class = self.elf_file.ehdr.class;
            let (sysv_hash_section, _) = self.elf_file.section_data(&sysv_hash_section_header).ok()?;
            let sysv_hash_section = sysv_hash_section.to_vec();
            let hash_section = SysVHashTable::new(elf_endianness, elf_class, &sysv_hash_section).ok()?;
            let (symbol_table, symbol_string_table) = self.elf_file.dynamic_symbol_table().ok()??;
            if let Some((_, symbol)) =
                hash_section.find(symbol_name.as_bytes(), &symbol_table, &symbol_string_table).ok()?
            {
                trace!(target: &self.name, r#"Found "{symbol_name}" in .hash"#);
                if include_overrides {
                    if let Some(&overridden_value) = self.symbol_overrides.get(symbol_name) {
                        let address = overridden_value.unwrap_or(UNDEFINED_SYMBOL_VALUE);
                        #[cfg(target_pointer_width = "64")]
                        trace!(target: &self.name, r#"Found override {:#018x} for "{}""#, address, symbol_name);
                        #[cfg(not(target_pointer_width = "64"))]
                        trace!(target: &self.name, r#"Found override {:#010x} for "{}""#, address, symbol_name);
                        return Some(LinkingSymbol::from_override(&symbol, Some(symbol_name.to_owned()), address));
                    }
                }
                return Some(LinkingSymbol::from(
                    &symbol,
                    Some(symbol_name.to_owned()),
                    self.mapping.base,
                    self.base_virtual_address,
                ));
            }
        }

        // TODO: Should be loop through our symbols using strcmp? It would be slow, but performance isn't a priority

        None
    }

    // Loop through our dependencies looking for a symbol
    // NOTE: This implementation is technically wrong, the spec says that the executable should be searched, then the
    //       symbols defined in the shared library, then the symbols in DT_NEEDED, then the DT_NEEDED of the first
    //       DT_NEEDED:                 1           JNI (ignore the executable consuming this library)
    //                              2-------3       JNI's DT_NEEDED
    //                            4---5     6       Each DT_NEEDED's dependencies
    //                                    7---8     And so on, descending the tree level by level
    fn find_global_symbol(&mut self, symbol_name: &str, include_overrides: bool) -> Option<LinkingSymbol> {
        if self.looking_for_symbol {
            return None;
        }
        self.looking_for_symbol = true;
        trace!(target: &self.name, "Looking for symbol {symbol_name}");
        for (_, dependency) in self.dependencies.iter() {
            if let Some(dependency) = dependency {
                // looking_for_symbol protects us from recursively calling lock()
                let mut dependency = dependency.lock().unwrap();
                let symbol = dependency
                    .find_local_symbol_by_name(symbol_name, include_overrides)
                    .or(dependency.find_global_symbol(symbol_name, include_overrides));
                if symbol.is_some() {
                    self.looking_for_symbol = false;
                    return symbol;
                }
            }
        }
        self.looking_for_symbol = false;
        None
    }

    pub(crate) fn plt_callback(&mut self, reloc_index: usize) -> Option<usize> {
        let mut relocation_offset = None;
        let mut relocation_addend = None;
        let mut relocation_symbol = None;

        if let Ok(Some(&rel_plt_header)) = self.elf_file.section_header_by_name(".rel.plt") {
            debug!(target: &self.name, "Trying to resolve .rel.plt[{}]", reloc_index);
            if let Ok(mut rel_plt) = self.elf_file.section_data_as_rels(&rel_plt_header) {
                if let Some(reloc) = rel_plt.nth(reloc_index) {
                    relocation_offset = Some(reloc.r_offset as usize);
                    relocation_addend = Some(0);
                    relocation_symbol = Some(reloc.r_sym);
                }
            }
        }
        if let Ok(Some(&rela_plt_header)) = self.elf_file.section_header_by_name(".rela.plt") {
            debug!(target: &self.name, "Trying to resolve .rela.plt[{}]", reloc_index);
            if let Ok(mut rela_plt) = self.elf_file.section_data_as_relas(&rela_plt_header) {
                if let Some(reloc) = rela_plt.nth(reloc_index) {
                    relocation_offset = Some(reloc.r_offset as usize);
                    relocation_addend = Some(reloc.r_addend);
                    relocation_symbol = Some(reloc.r_sym);
                }
            }
        }

        let relocation_offset = relocation_offset?;
        let relocation_addend = relocation_addend?;
        let relocation_symbol = relocation_symbol?;
        debug!(target: &self.name, "PLT relocation {reloc_index} is for symbol {relocation_symbol}");
        let symbol_addr = match self.resolve_plt_symbol(relocation_symbol) {
            Some(symbol) => Some(symbol),
            None => {
                error!(target: &self.name, "Failed to resolve PLT symbol {relocation_symbol}. We are probably about to crash");
                None
            },
        }?;
        let target_addr = self.get_offset(relocation_offset);
        let target_value = add_addend(symbol_addr, relocation_addend);
        #[cfg(target_pointer_width = "64")]
        debug!(target: &self.name, "Handling PLT entry {reloc_index} by writing {:#018x} to {:#018x}", target_value, target_addr);
        #[cfg(not(target_pointer_width = "64"))]
        debug!(target: &self.name, "Handling PLT entry {reloc_index} by writing {:#010x} to {:#010x}", target_value, target_addr);
        unsafe { *(target_addr as *mut usize) = target_value }
        Some(symbol_addr)
    }

    fn resolve_plt_symbol(&mut self, symbol_idx: u32) -> Option<usize> {
        let local_symbol = self.find_local_symbol_by_index(symbol_idx, true)?;
        if let Some(ref local_symbol_name) = local_symbol.name {
            debug!(target: &self.name, "Found name '{local_symbol_name}' for PLT symbol {symbol_idx}");
        }
        if local_symbol.address.is_some() {
            return local_symbol.address;
        }
        if local_symbol.value != 0 {
            return Some(self.get_offset(local_symbol.value as usize));
        }
        let symbol_name = local_symbol.name?;
        let global_symbol = self.find_global_symbol(&symbol_name, true)?;
        if global_symbol.address.is_some() {
            return global_symbol.address;
        }
        match global_symbol.value {
            0 => None,
            value => Some(self.get_offset(value as usize)),
        }
    }
}

struct Relocation {
    offset: usize,
    rel_type: u32,
    symbol: u32,
    addend: i64,
}
impl From<Rel> for Relocation {
    fn from(relocation: Rel) -> Self {
        Self { offset: relocation.r_offset as usize, rel_type: relocation.r_type, symbol: relocation.r_sym, addend: 0 }
    }
}
impl From<Rela> for Relocation {
    fn from(relocation: Rela) -> Self {
        Self {
            offset: relocation.r_offset as usize,
            rel_type: relocation.r_type,
            symbol: relocation.r_sym,
            addend: relocation.r_addend,
        }
    }
}
impl Debug for Relocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(target_pointer_width = "64")]
        let ret = write!(
            f,
            "Relocation {{ offset: {:#018x} type: {:#010x} symbol: {} addend: {} }}",
            self.offset, self.rel_type, self.symbol, self.addend
        );
        #[cfg(not(target_pointer_width = "64"))]
        let ret = write!(
            f,
            "Relocation {{ offset: {:#010x} type: {:#010x} symbol: {} addend: {} }}",
            self.offset, self.rel_type, self.symbol, self.addend
        );
        ret
    }
}

// Used to represent a symbol while linking
#[allow(dead_code)]
struct LinkingSymbol {
    name: Option<String>,
    shndx: u16,
    value: u64,
    address: Option<usize>,
    size: u64,
    sym_type: u8,
    binding: u8,
    visibility: u8,
}

impl LinkingSymbol {
    pub fn from(symbol: &Symbol, name: Option<String>, mapping_base: usize, virtual_base_address: usize) -> Self {
        LinkingSymbol {
            name,
            shndx: symbol.st_shndx,
            value: symbol.st_value,
            address: match symbol.st_value {
                STN_UNDEF => None,
                value => Some(mapping_base + value as usize - virtual_base_address),
            },
            size: symbol.st_size,
            sym_type: symbol.st_symtype(),
            binding: symbol.st_bind(),
            visibility: symbol.st_vis(),
        }
    }

    pub fn from_override(symbol: &Symbol, name: Option<String>, address: usize) -> Self {
        LinkingSymbol {
            name,
            shndx: symbol.st_shndx,
            value: symbol.st_value,
            address: Some(address),
            size: symbol.st_size,
            sym_type: symbol.st_symtype(),
            binding: symbol.st_bind(),
            visibility: symbol.st_vis(),
        }
    }
}

fn add_addend(addr: usize, addend: i64) -> usize {
    if addend.is_negative() {
        addr - (addend.unsigned_abs() as usize)
    } else {
        addr + (addend as usize)
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("failed to find file")]
    FileNotFound,
    #[error("failed to open file")]
    FailedToOpen,
    #[error("the file is not a shared object file")]
    NotDynamicObject,
    #[error("failed to map memory - {0}")]
    MemoryMapFailed(String),
    #[error("failed to find .dynamic section")]
    NoDyanmicSection,
}
