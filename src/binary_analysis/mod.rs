use std::fs::File;
use std::path::Path;

use object::{File as ObjectFile, Object, ObjectSection, ObjectSymbol, Section, SymbolSection};
use rustc_middle::ty::TyCtxt;

mod build_error;
mod dwarf;
mod reconstruct;

pub fn binary_analysis<'tcx>(tcx: TyCtxt<'tcx>, path: &Path) {
    let file = File::open(path).unwrap();
    let mmap = unsafe { rustc_data_structures::memmap::Mmap::map(file) }.unwrap();
    let object = ObjectFile::parse(&*mmap).unwrap();

    build_error::build_error_detection(tcx, &object);
}

fn find_symbol_from_section_offset<'obj>(
    file: &ObjectFile<'obj>,
    section: &Section<'_, 'obj>,
    offset: u64,
) -> Option<(&'obj str, u64)> {
    let section_needle = SymbolSection::Section(section.index());
    for sym in file.symbols() {
        if sym.section() != section_needle {
            continue;
        }

        let start = sym.address();
        let end = start + sym.size();
        if (start..end).contains(&offset) {
            if let Ok(name) = sym.name() {
                return Some((name, offset - start));
            }
        }
    }

    None
}
