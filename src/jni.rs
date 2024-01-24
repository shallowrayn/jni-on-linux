use std::{collections::HashMap, fs::File, path::PathBuf, rc::Rc, sync::Mutex};

use elf::{
    abi::{DT_NEEDED, DT_RUNPATH, ET_DYN, PT_LOAD},
    endian::AnyEndian,
    hash::{GnuHashTable, SysVHashTable},
    symbol::Symbol,
    ElfStream,
};
use thiserror::Error;

use super::{locate, mmap::MemoryMapping};

pub struct JNI {
    path: PathBuf,
    elf_file: ElfStream<AnyEndian, File>,
    mapping: MemoryMapping,
    base_virtual_address: usize, // Lowest PT_LOAD virtual address
    dependencies: HashMap<String, Option<Rc<Mutex<JNI>>>>,
    loaded_dependencies: bool,
    have_been_initialized: bool,
    looking_for_symbol: bool,
}

impl JNI {
    pub fn new(path: PathBuf) -> Result<Self, Error> {
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
        let mapping = match MemoryMapping::new(mapping_file, elf_file.segments()) {
            Ok(mapping) => mapping,
            Err(error) => return Err(Error::MemoryMapFailed(error)),
        };
        let base_virtual_address = elf_file.segments().iter().find(|&s| s.p_type == PT_LOAD).unwrap().p_vaddr as usize;
        Ok(Self {
            path,
            elf_file,
            mapping,
            base_virtual_address,
            dependencies: HashMap::new(),
            loaded_dependencies: false,
            have_been_initialized: false,
            looking_for_symbol: false,
        })
    }

    pub fn new_from_name(name: &str) -> Result<Self, Error> {
        match locate::locate_library(name, None) {
            Some(lib_path) => Self::new(lib_path),
            None => Err(Error::FileNotFound),
        }
    }

    pub fn add_dependency(&mut self, name: &str, lib: Option<JNI>) {
        self.dependencies.insert(name.to_string(), lib.map(Mutex::new).map(Rc::new));
    }

    pub fn add_shared_dependency(&mut self, name: &str, lib: Option<Rc<Mutex<JNI>>>) {
        self.dependencies.insert(name.to_string(), lib);
    }

    pub fn load_dependencies(&mut self) {
        if self.loaded_dependencies {
            return;
        }
        self.loaded_dependencies = true;

        // Dependencies are stored using DT_NEEDED keys in the .dynamic section. We also need DT_RUNPATH for locating
        let Ok(Some(dynamic_section)) = self.elf_file.dynamic() else {
            return;
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
            return;
        };
        let dynamic_string_table_header = *dynamic_string_table_header; // End mutable borrow of self.elf_file
        let Ok(dynamic_string_table) = self.elf_file.section_data_as_strtab(&dynamic_string_table_header) else {
            return;
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
            if self.dependencies.contains_key(&lib_name) {
                continue;
            }
            match locate::locate_library_internal(&lib_name, None, parent_dir.clone(), dt_runpath.clone()) {
                Some(lib_path) => {
                    self.dependencies.insert(lib_name, JNI::new(lib_path).ok().map(Mutex::new).map(Rc::new));
                },
                None => {
                    self.dependencies.insert(lib_name, None);
                },
            }
        }
    }

    pub fn initialize(&mut self) {
        if self.have_been_initialized {
            return;
        }
        self.have_been_initialized = true;

        for (_, dependency) in self.dependencies.iter() {
            if let Some(dependency) = dependency {
                // NOTE - Deadlocks
                // The guard at the top of this function prevents this loop being recursively executed on one instance
                // If A depends on B and A and B both depend on C, C's lock will be released before B::initialize()
                dependency.lock().unwrap().initialize();
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
        struct Relocation {
            offset: usize,
            rel_type: u32,
            symbol: u32,
            addend: i64,
        }
        let mut relocations = Vec::new();
        if let Ok(Some(rel_dyn_header)) = self.elf_file.section_header_by_name(".rel.dyn") {
            let rel_dyn_header = *rel_dyn_header;
            if let Ok(rel_dyn) = self.elf_file.section_data_as_rels(&rel_dyn_header) {
                for relocation in rel_dyn {
                    let reloc = Relocation {
                        offset: relocation.r_offset as usize,
                        rel_type: relocation.r_type,
                        symbol: relocation.r_sym,
                        addend: 0,
                    };
                    relocations.push(reloc);
                }
            }
        }
        if let Ok(Some(rela_dyn_header)) = self.elf_file.section_header_by_name(".rela.dyn") {
            let rela_dyn_header = *rela_dyn_header;
            if let Ok(rela_dyn) = self.elf_file.section_data_as_relas(&rela_dyn_header) {
                for relocation in rela_dyn {
                    let reloc = Relocation {
                        offset: relocation.r_offset as usize,
                        rel_type: relocation.r_type,
                        symbol: relocation.r_sym,
                        addend: relocation.r_addend,
                    };
                    relocations.push(reloc);
                }
            }
        }
        for relocation in relocations {
            let target_addr = self.mapping.base + relocation.offset - self.base_virtual_address;
            // Only some relocations need the symbol
            macro_rules! reloc_needs_symbol {
                ($reloc:expr) => {{
                    let symbol = if relocation.symbol != 0 {
                        let local_symbol =
                            self.find_local_symbol_by_index(relocation.symbol).expect("Failed to find symbol");
                        if local_symbol.value != 0 {
                            Some(local_symbol)
                        } else {
                            let symbol_name = local_symbol.name.expect("Cannot lookup symbol without name");
                            self.find_global_symbol(&symbol_name)
                        }
                    } else {
                        None
                    };
                    match symbol {
                        Some(s) => self.mapping.base + s.value as usize - self.base_virtual_address,
                        None => panic!("Reloc {} failed to find symbol", $reloc),
                    }
                }};
            }
            // Deal with addend
            fn add_addend(addr: usize, addend: i64) -> usize {
                if addend.is_negative() {
                    addr - (addend.unsigned_abs() as usize)
                } else {
                    addr + (addend as usize)
                }
            }

            #[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
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
            #[cfg(not(all(target_arch = "x86_64", target_pointer_width = "64")))]
            panic!("Unhandled system architecture")
        }
    }

    // Look for a local symbol using its index
    fn find_local_symbol_by_index(&mut self, index: u32) -> Option<LinkingSymbol> {
        let (symbol_table, symbol_string_table) = self.elf_file.dynamic_symbol_table().ok()??;
        let symbol = symbol_table.get(index as usize).ok()?;
        let mut symbol_name = None;
        if symbol.st_name != 0 {
            symbol_name = Some(symbol_string_table.get(symbol.st_name as usize).ok()?.to_owned());
        }
        Some(LinkingSymbol::from(&symbol, symbol_name))
    }

    // Look for a local symbol using the hash tables
    fn find_local_symbol_by_name(&mut self, symbol_name: &str) -> Option<LinkingSymbol> {
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
                return Some(LinkingSymbol::from(&symbol, Some(symbol_name.to_owned())));
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
                return Some(LinkingSymbol::from(&symbol, Some(symbol_name.to_owned())));
            }
        }

        // TODO: Should be loop through our symbols using strcmp? It would be slow, but performance isn't a priority

        None
    }

    // Loop through our dependencies looking for a symbol
    fn find_global_symbol(&mut self, symbol_name: &str) -> Option<LinkingSymbol> {
        if self.looking_for_symbol {
            return None;
        }
        self.looking_for_symbol = true;
        for (_, dependency) in self.dependencies.iter() {
            if let Some(dependency) = dependency {
                // looking_for_symbol protects us from recursively calling lock()
                let mut dependency = dependency.lock().unwrap();
                let symbol =
                    dependency.find_local_symbol_by_name(symbol_name).or(dependency.find_global_symbol(symbol_name));
                if symbol.is_some() {
                    return symbol;
                }
            }
        }
        self.looking_for_symbol = false;
        None
    }

    // First check our symbols, then look through dependencies
    fn find_symbol_by_name(&mut self, symbol_name: &str) -> Option<LinkingSymbol> {
        let symbol = self.find_local_symbol_by_name(symbol_name);
        if symbol.is_some() {
            return symbol;
        }
        self.find_global_symbol(symbol_name)
    }
}

// Used to represent a symbol while linking
struct LinkingSymbol {
    name: Option<String>,
    shndx: u16,
    value: u64,
    size: u64,
    sym_type: u8,
    binding: u8,
    visibility: u8,
}

impl LinkingSymbol {
    pub fn from(symbol: &Symbol, name: Option<String>) -> Self {
        LinkingSymbol {
            name,
            shndx: symbol.st_shndx,
            value: symbol.st_value,
            size: symbol.st_size,
            sym_type: symbol.st_symtype(),
            binding: symbol.st_bind(),
            visibility: symbol.st_vis(),
        }
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
}
