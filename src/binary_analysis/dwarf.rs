use std::num::NonZero;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::{borrow::Cow, collections::BTreeMap};

use gimli::{
    AttributeValue, DebuggingInformationEntry, Dwarf, EndianSlice, LineProgramHeader, LineRow, Unit,
};
use object::Object;
use object::{
    Endian, File, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget, Section,
    SectionIndex, elf::SHF_ALLOC,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Object(#[from] object::Error),
    #[error("{0}")]
    Gimli(gimli::Error),
    #[error("unexpected ELF information: {0}")]
    UnexpectedElf(&'static str),
    #[error("unexpected DWARF information: {0}")]
    UnexpectedDwarf(&'static str),
    #[error("{0}")]
    Other(&'static str),
}

impl From<gimli::Error> for Error {
    fn from(value: gimli::Error) -> Self {
        Self::Gimli(value)
    }
}

// Section address encoder and decoder.
//
// `gimli` library does not natively handle relocations; this is fine for binaries, but for relocatable
// object files, sections begin with address 0 and you cannot tell apart different sections by looking at address.
//
// To solve this, we lay all sections flat in memory (with some gaps between in cases there are pointers going beyond section boundaries).
// We can then use these offsets to provide revese lookup to determine the symbolic addresses.
struct SectionLayout {
    forward_map: Vec<(u64, u64)>,
    reverse_map: BTreeMap<u64, usize>,
}

impl SectionLayout {
    const SECTION_GAP: u64 = 65536;

    fn for_object<'data>(object: &File<'data>) -> Result<Self, Error> {
        fn section_alloc(section: &Section<'_, '_>) -> bool {
            match section.flags() {
                object::SectionFlags::None => false,
                object::SectionFlags::Elf { sh_flags } => sh_flags & SHF_ALLOC as u64 != 0,
                _ => bug!(),
            }
        }

        let section_count = object
            .sections()
            .map(|x| x.index().0)
            .max()
            .unwrap_or_default()
            + 1;

        // All non-allocate sections go to address 0, where we pre-allocate based on the maximum size.
        let unalloc_sections_max = object
            .sections()
            .filter(section_alloc)
            .map(|x| x.size())
            .max()
            .unwrap_or(0);

        let overflow_err =
            || Error::UnexpectedElf("cannot lay all sections in 64-bit address space");

        let mut allocated = unalloc_sections_max
            .checked_add(Self::SECTION_GAP)
            .ok_or_else(overflow_err)?;

        let mut forward_map = vec![(0, 0); section_count];
        let mut reverse_map = BTreeMap::new();

        for section in object.sections() {
            let index = section.index();
            if !section_alloc(&section) {
                forward_map[index.0] = (0, section.size());
                continue;
            }

            let address = allocated
                .checked_next_multiple_of(section.align())
                .ok_or_else(overflow_err)?;
            forward_map[index.0] = (address, section.size());
            reverse_map.insert(address, index.0);

            allocated = address
                .checked_add(section.size())
                .ok_or_else(overflow_err)?
                .checked_add(Self::SECTION_GAP)
                .ok_or_else(overflow_err)?;
        }

        if allocated
            .checked_add(Self::SECTION_GAP)
            .ok_or_else(overflow_err)?
            > i64::MAX as u64
        {
            Err(overflow_err())?;
        }

        Ok(SectionLayout {
            forward_map,
            reverse_map,
        })
    }

    fn encode(&self, section: SectionIndex, offset: i64) -> Result<u64, Error> {
        let (address, size) = self.forward_map[section.0];
        if offset < -(Self::SECTION_GAP as i64 / 2)
            || offset > (size + Self::SECTION_GAP / 2) as i64
        {
            Err(Error::UnexpectedElf("symbol offset too big"))?
        }

        Ok(address.wrapping_add(offset as _))
    }

    fn decode(&self, address: u64) -> Result<(SectionIndex, i64), Error> {
        let address_plus_gap = address
            .checked_add(Self::SECTION_GAP / 2)
            .ok_or(Error::UnexpectedElf("unexpected symbol offset"))?;
        let Some((&section_start, &index)) = self.reverse_map.range(..address_plus_gap).next_back()
        else {
            Err(Error::UnexpectedElf(
                "address from unallocated section cannot be decoded",
            ))?
        };

        let offset = (address as i64).wrapping_sub(section_start as _);
        assert_eq!(self.encode(SectionIndex(index), offset).unwrap(), address);
        Ok((SectionIndex(index), offset))
    }
}

fn load_section<'file, 'data>(
    object: &'file File<'data>,
    layout: &SectionLayout,
    name: &str,
) -> Result<Cow<'data, [u8]>, Error> {
    let Some(section) = object.section_by_name(name) else {
        return Ok(Cow::Borrowed(&[]));
    };

    let mut data = section.uncompressed_data()?;

    for (offset, reloc) in section.relocations() {
        let data_mut = data.to_mut();
        if reloc.kind() != RelocationKind::Absolute {
            Err(Error::UnexpectedElf(
                "non-absolute relocation kind found in DWARF section",
            ))?
        }

        let (symbol_section_index, symbol_offset) = match reloc.target() {
            RelocationTarget::Symbol(symbol) => {
                let symbol = object
                    .symbol_by_index(symbol)
                    .map_err(|_| Error::UnexpectedElf("symbol not found"))?;

                let Some(section_index) = symbol.section().index() else {
                    Err(Error::UnexpectedElf(
                        "symbol is not associated with a section",
                    ))?
                };

                (section_index, symbol.address())
            }
            RelocationTarget::Section(section_index) => (section_index, 0),
            RelocationTarget::Absolute | _ => Err(Error::UnexpectedElf(
                "absolute relocation target found in DWARF section",
            ))?,
        };

        let symbol_section = object
            .section_by_index(symbol_section_index)
            .map_err(|_| Error::UnexpectedElf("section not found"))?;

        if symbol_section.address() != 0 {
            Err(Error::UnexpectedElf(
                "section address is non-zero in a relocatable file",
            ))?
        }

        let address = layout.encode(symbol_section_index, symbol_offset as _)?;
        let value = reloc.addend().wrapping_add(address as _);

        match reloc.size() {
            32 => {
                let addend = if reloc.has_implicit_addend() {
                    i32::from_le_bytes(data_mut[offset as usize..][..4].try_into().unwrap())
                } else {
                    0
                };
                let value: i32 = value
                    .wrapping_add(addend as i64)
                    .try_into()
                    .map_err(|_| Error::UnexpectedElf("relocation truncated to fit"))?;

                data_mut[offset as usize..][..4].copy_from_slice(&value.to_le_bytes());
            }
            64 => {
                let addend = if reloc.has_implicit_addend() {
                    i64::from_le_bytes(data_mut[offset as usize..][..8].try_into().unwrap())
                } else {
                    0
                };
                let value = value.wrapping_add(addend as i64);
                data_mut[offset as usize..][..8].copy_from_slice(&value.to_le_bytes());
            }
            _ => Err(Error::UnexpectedElf("unknown relocation size"))?,
        }
    }

    Ok(data)
}

type ReaderTy<'a> = EndianSlice<'a, gimli::LittleEndian>;

pub struct DwarfLoader<'file, 'data> {
    section_layout: SectionLayout,
    // This is actually `Dwarf<ReaderTy<'dwarf_sections>`.
    dwarf: Dwarf<ReaderTy<'file>>,
    #[allow(unused)]
    dwarf_sections: Arc<gimli::DwarfSections<Cow<'data, [u8]>>>,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub file: PathBuf,
    pub line: u64,
    pub column: u64,
}

#[derive(Debug)]
pub struct Call {
    pub caller: String,
    pub callee: String,
    pub location: Option<Location>,
}

impl<'file, 'data> DwarfLoader<'file, 'data> {
    pub fn new(object: &'file File<'data>) -> Result<Self, Error> {
        if !object.endianness().is_little_endian() {
            Err(Error::UnexpectedElf(
                "only little endian object files are supported",
            ))?
        }

        let section_layout = SectionLayout::for_object(object)?;

        let dwarf_sections = Arc::new(gimli::DwarfSections::load(|id| {
            load_section(object, &&section_layout, id.name())
        })?);
        let dwarf =
            dwarf_sections.borrow(|section| gimli::EndianSlice::new(&section, gimli::LittleEndian));
        // SAFETY: erase lifetime. This is fine as `dwarf` will be dropped before `dwarf_sections`.
        let dwarf_transmute =
            unsafe { std::mem::transmute::<Dwarf<ReaderTy<'_>>, Dwarf<ReaderTy<'_>>>(dwarf) };

        Ok(Self {
            section_layout,
            dwarf: dwarf_transmute,
            dwarf_sections,
        })
    }

    // This returns the correct lifetime instead of the hacked one.
    fn dwarf(&self) -> &Dwarf<ReaderTy<'_>> {
        &self.dwarf
    }

    /// Obtain the linkage name of a subprogram or inlined subroutine.
    fn linkage_name(
        &self,
        unit: &Unit<ReaderTy<'_>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_>>,
    ) -> Result<String, Error> {
        let mut attrs = die.attrs();
        let mut name = None;
        let mut deleg = None;

        while let Some(attr) = attrs.next()? {
            match attr.name() {
                gimli::DW_AT_linkage_name => {
                    return Ok(self
                        .dwarf()
                        .attr_string(unit, attr.value())?
                        .to_string()?
                        .to_owned());
                }

                gimli::DW_AT_name => {
                    name = Some(
                        self.dwarf()
                            .attr_string(unit, attr.value())?
                            .to_string()?
                            .to_owned(),
                    );
                }

                gimli::DW_AT_abstract_origin | gimli::DW_AT_specification => {
                    // Delegation
                    deleg = Some(attr.value());
                }

                _ => (),
            }
        }

        if let Some(name) = name {
            return Ok(name);
        }

        let Some(refer) = deleg else {
            Err(Error::UnexpectedDwarf(
                "Cannot find name for DW_TAG_subprogram",
            ))?
        };

        match refer {
            AttributeValue::UnitRef(offset) => {
                let mut entries = unit.entries_at_offset(offset)?;
                entries
                    .next_entry()?
                    .ok_or(Error::UnexpectedDwarf("Referenced entry not found"))?;

                let next_die = entries.current().unwrap();
                return self.linkage_name(unit, next_die);
            }

            _ => Err(Error::UnexpectedDwarf("Unsupported reference type"))?,
        }
    }

    /// Obtain PC ranges related to a DIE.
    fn ranges(
        &self,
        unit: &Unit<ReaderTy<'_>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_>>,
    ) -> Result<(SectionIndex, Vec<Range<i64>>), Error> {
        let mut ranges = Vec::new();

        let mut attrs = die.attrs();
        while let Some(attr) = attrs.next()? {
            match attr.name() {
                gimli::DW_AT_low_pc => {
                    let Some(low_pc) = self.dwarf().attr_address(&unit, attr.value())? else {
                        Err(Error::UnexpectedDwarf("DW_AT_low_pc is not an address"))?
                    };

                    let Some(high_pc) = die.attr_value(gimli::DW_AT_high_pc)? else {
                        Err(Error::UnexpectedDwarf(
                            "DW_AT_high_pc not found at DW_TAG_inlined_subroutine",
                        ))?
                    };

                    let Some(high_pc) = high_pc.udata_value() else {
                        Err(Error::UnexpectedDwarf("DW_AT_high_pc is not udata"))?
                    };

                    ranges.push((low_pc, high_pc));
                }

                // This is handled by DW_AT_low_pc.
                gimli::DW_AT_high_pc => (),

                gimli::DW_AT_ranges => {
                    let AttributeValue::DebugRngListsIndex(offset) = attr.value() else {
                        dbg!(attr);
                        Err(Error::UnexpectedDwarf(
                            "DW_AT_ranges is not rnglist reference",
                        ))?
                    };

                    let offset = self.dwarf().ranges_offset(unit, offset)?;
                    let mut range = self.dwarf().ranges(unit, offset)?;
                    while let Some(range) = range.next()? {
                        ranges.push((range.begin, range.end.wrapping_sub(range.begin)));
                    }
                }

                _ => (),
            }
        }

        if ranges.is_empty() {
            return Ok((SectionIndex(0), Vec::new()));
        }

        let encoded_section = self.section_layout.decode(ranges[0].0)?.0;

        let ranges = ranges
            .into_iter()
            .map(|(begin, len)| {
                let (sec, begin) = self.section_layout.decode(begin)?;
                if sec != encoded_section {
                    return Err(Error::UnexpectedDwarf(
                        "Single DIE covers multiple sections",
                    ));
                }

                Ok(begin..begin.wrapping_add(len as _))
            })
            .collect::<Result<_, _>>()?;

        Ok((encoded_section, ranges))
    }

    fn call_location(
        &self,
        unit: &Unit<ReaderTy<'_>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_>>,
    ) -> Result<Option<Location>, Error> {
        let Some(file) = die.attr(gimli::DW_AT_call_file)? else {
            // This may happen when two calls from different files are merged.
            return Ok(None);
        };

        let file = self.file_name(
            unit,
            unit.line_program
                .as_ref()
                .ok_or(Error::UnexpectedDwarf("line number table not present"))?
                .header(),
            file.udata_value()
                .ok_or(Error::UnexpectedDwarf("file number is not udata"))?,
        )?;

        let Some(line) = die.attr(gimli::DW_AT_call_line)? else {
            // This may happen when two calls from different lines are merged.
            return Ok(None);
        };
        let line = line
            .udata_value()
            .ok_or(Error::UnexpectedDwarf("line number is not udata"))?;

        let column = match die.attr(gimli::DW_AT_call_column)? {
            None => 0,
            Some(column) => column
                .udata_value()
                .ok_or(Error::UnexpectedDwarf("column number is not udata"))?,
        };

        Ok(Some(Location { file, line, column }))
    }

    pub fn inline_info<'tcx>(
        &self,
        section_index: SectionIndex,
        offset: u64,
    ) -> Result<Vec<Call>, Error> {
        let mut iter = self.dwarf.units();
        let mut callstack = Vec::<Call>::new();

        while let Some(header) = iter.next()? {
            let unit = self.dwarf.unit(header)?;

            let mut stack = Vec::new();
            let mut entries = unit.entries();
            while let Some((depth, die)) = entries.next_dfs()? {
                for _ in depth..=0 {
                    stack.pop();
                }

                if matches!(
                    die.tag(),
                    gimli::DW_TAG_subprogram | gimli::DW_TAG_inlined_subroutine
                ) {
                    stack.push(Some(self.linkage_name(&unit, die)?));
                } else {
                    stack.push(None);
                }

                if die.tag() != gimli::DW_TAG_inlined_subroutine {
                    continue;
                }

                let ranges = self.ranges(&unit, die)?;
                if ranges.0 != section_index {
                    continue;
                }

                if ranges
                    .1
                    .iter()
                    .any(|range| range.contains(&(offset as i64)))
                {
                    let callee = stack.last().unwrap().as_ref().unwrap();
                    let caller = stack
                        .iter()
                        .rev()
                        .skip(1)
                        .find(|x| x.is_some())
                        .and_then(|x| x.as_ref())
                        .ok_or(Error::UnexpectedDwarf(
                            "DW_TAG_inlined_subroutine is not nested inside DW_TAG_subprogram",
                        ))?;

                    // Call stack must form a chain.
                    if let Some(last_call) = callstack.last() {
                        if last_call.callee != *caller {
                            Err(Error::UnexpectedDwarf("Inlined call does not form a chain"))?
                        }
                    }

                    let location = self.call_location(&unit, die)?;
                    callstack.push(Call {
                        caller: caller.clone(),
                        callee: callee.clone(),
                        location,
                    });
                }
            }
        }

        Ok(callstack)
    }

    fn file_name(
        &self,
        unit: &Unit<ReaderTy<'_>>,
        line: &LineProgramHeader<ReaderTy<'_>>,
        index: u64,
    ) -> Result<PathBuf, Error> {
        let file = line.file(index).ok_or(Error::UnexpectedDwarf(
            "debug_lines referenced non-existent file",
        ))?;

        let mut path = PathBuf::new();

        if file.directory_index() != 0 {
            let directory = file.directory(line).ok_or(Error::UnexpectedDwarf(
                "debug_lines referenced non-existent directory",
            ))?;

            path.push(self.dwarf().attr_string(&unit, directory)?.to_string()?);
        }

        path.push(
            self.dwarf()
                .attr_string(&unit, file.path_name())?
                .to_string()?,
        );

        Ok(path)
    }

    pub fn locate<'tcx>(
        &self,
        section_index: SectionIndex,
        offset: u64,
    ) -> Result<Option<Location>, Error> {
        // FIXME: should this be optimized?

        let mut iter = self.dwarf.units();
        while let Some(header) = iter.next()? {
            let mut unit = self.dwarf.unit(header)?;

            let mut prev: Option<(_, LineRow)> = None;
            if let Some(ilnp) = unit.line_program.take() {
                let mut rows = ilnp.rows();
                while let Some((_, row)) = rows.next_row()? {
                    let row = *row;
                    let encoded_address = row.address();
                    let (encoded_section, encoded_offset) =
                        self.section_layout.decode(encoded_address)?;

                    // Skip over sections that we don't care about.
                    if encoded_section != section_index {
                        continue;
                    }

                    if let Some((prev_addr, prev_row)) = prev
                        && prev_row.line().is_some()
                        && (prev_addr..encoded_offset).contains(&(offset as i64))
                    {
                        let file = self.file_name(&unit, rows.header(), prev_row.file_index())?;
                        let line = prev_row.line().map_or(0, NonZero::get);
                        let column = match prev_row.column() {
                            gimli::ColumnType::LeftEdge => 0,
                            gimli::ColumnType::Column(v) => v.get(),
                        };
                        return Ok(Some(Location { file, line, column }));
                    }

                    if row.end_sequence() {
                        prev = None;
                    } else {
                        prev = Some((encoded_offset, row));
                    }
                }
            }
        }

        Ok(None)
    }
}
