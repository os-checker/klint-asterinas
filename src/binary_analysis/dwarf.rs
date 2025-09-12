use std::borrow::Cow;
use std::num::NonZero;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gimli::{
    AttributeValue, DebuggingInformationEntry, Dwarf, EndianSlice, LineProgramHeader, LineRow,
    Reader, RelocateReader, Unit,
};
use object::{
    Endian, File, Object, ObjectSection, ObjectSymbol, Relocation, RelocationKind,
    RelocationTarget, Section, SectionIndex,
};
use rustc_data_structures::fx::FxHashMap;
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

// Section address encoding.
//
// `gimli` library does not natively handle relocations; this is fine for binaries, but for relocatable
// object files, sections begin with address 0 and you cannot tell apart different sections by looking at address.
//
// We use a clever scheme to encode the section index into higher bits of the address. This assumes that no single section
// will be larger than 2GiB which should be a sane assumption to make.

fn encode_address(section: SectionIndex, offset: i64) -> u64 {
    (section.0 as u64) << 32 | 0x80000000 | (offset as u64)
}

fn decode_address(address: u64) -> (SectionIndex, i64) {
    let section = SectionIndex((address >> 32) as _);
    (
        section,
        address.wrapping_sub(encode_address(section, 0)) as _,
    )
}

struct SectionRelocate<'file, 'data> {
    object: &'file File<'data>,
    map: FxHashMap<u64, Relocation>,
}

impl std::fmt::Debug for SectionRelocate<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SectionRelocate").finish()
    }
}

impl<'file, 'data> SectionRelocate<'file, 'data> {
    fn for_section(
        object: &'file File<'data>,
        section: &Section<'data, 'file>,
    ) -> Result<Self, Error> {
        Ok(Self {
            object,
            map: section
                .relocations()
                .map(|(offset, reloc)| {
                    if reloc.kind() != RelocationKind::Absolute {
                        Err(Error::UnexpectedElf(
                            "non-absolute relocation kind found in DWARF section",
                        ))?
                    }

                    match reloc.target() {
                        RelocationTarget::Symbol(symbol) => {
                            let symbol = object
                                .symbol_by_index(symbol)
                                .map_err(|_| Error::UnexpectedElf("symbol not found"))?;

                            let Some(section_index) = symbol.section().index() else {
                                Err(Error::UnexpectedElf(
                                    "symbol is not associated with a section",
                                ))?
                            };

                            let section = object
                                .section_by_index(section_index)
                                .map_err(|_| Error::UnexpectedElf("section not found"))?;

                            if section.address() != 0 {
                                Err(Error::UnexpectedElf(
                                    "section address is non-zero in a relocatable file",
                                ))?
                            }
                        }
                        RelocationTarget::Section(section_index) => {
                            let section = object
                                .section_by_index(section_index)
                                .map_err(|_| Error::UnexpectedElf("section not found"))?;

                            if section.address() != 0 {
                                Err(Error::UnexpectedElf(
                                    "section address is non-zero in a relocatable file",
                                ))?
                            }
                        }
                        RelocationTarget::Absolute | _ => Err(Error::UnexpectedElf(
                            "absolute relocation target found in DWARF section",
                        ))?,
                    }

                    Ok((offset, reloc))
                })
                .collect::<Result<FxHashMap<_, _>, Error>>()?,
        })
    }
}

impl<'file, 'data> gimli::Relocate for &SectionRelocate<'file, 'data> {
    fn relocate_address(&self, offset: usize, value: u64) -> gimli::Result<u64> {
        let Some(reloc) = self.map.get(&(offset as u64)) else {
            return Ok(value);
        };

        let addend = match reloc.target() {
            RelocationTarget::Symbol(symbol) => {
                let symbol = self.object.symbol_by_index(symbol).unwrap();
                let section_index = symbol.section().index().unwrap();
                encode_address(
                    section_index,
                    (symbol.address() as i64).wrapping_add(reloc.addend()),
                )
            }
            RelocationTarget::Section(section_index) => {
                encode_address(section_index, reloc.addend())
            }
            _ => unreachable!(),
        };

        Ok(if reloc.has_implicit_addend() {
            value.wrapping_add(addend)
        } else {
            addend
        })
    }

    fn relocate_offset(&self, offset: usize, value: usize) -> gimli::Result<usize> {
        let Some(reloc) = self.map.get(&(offset as u64)) else {
            return Ok(value);
        };

        let addend = match reloc.target() {
            RelocationTarget::Symbol(symbol) => {
                let symbol = self.object.symbol_by_index(symbol).unwrap();
                symbol.address().wrapping_add(reloc.addend() as u64)
            }
            RelocationTarget::Section(_) => reloc.addend() as u64,
            _ => unreachable!(),
        };

        <usize as gimli::ReaderOffset>::from_u64(if reloc.has_implicit_addend() {
            (value as u64).wrapping_add(addend)
        } else {
            addend
        })
    }
}

struct SectionWithReloc<'file, 'data> {
    data: Cow<'data, [u8]>,
    reloc: SectionRelocate<'file, 'data>,
}

fn load_section<'file, 'data>(
    object: &'file File<'data>,
    name: &str,
) -> Result<SectionWithReloc<'file, 'data>, Error> {
    let Some(section) = object.section_by_name(name) else {
        return Ok(SectionWithReloc {
            data: Cow::Borrowed(&[]),
            reloc: SectionRelocate {
                object: object,
                map: Default::default(),
            },
        });
    };

    Ok(SectionWithReloc {
        data: section.uncompressed_data()?,
        reloc: SectionRelocate::for_section(object, &section)?,
    })
}

type ReaderTy<'a, 'file, 'data> =
    RelocateReader<EndianSlice<'a, gimli::LittleEndian>, &'a SectionRelocate<'file, 'data>>;

pub struct DwarfLoader<'file, 'data> {
    // This is actually `Dwarf<ReaderTy<'dwarf_sections, 'file, 'data>`.
    dwarf: Dwarf<ReaderTy<'file, 'file, 'data>>,
    #[allow(unused)]
    dwarf_sections: Arc<gimli::DwarfSections<SectionWithReloc<'file, 'data>>>,
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

        let dwarf_sections = Arc::new(gimli::DwarfSections::load(|id| {
            load_section(object, id.name())
        })?);
        let dwarf = dwarf_sections.borrow(|section| {
            let slice = gimli::EndianSlice::new(&section.data, gimli::LittleEndian);
            gimli::RelocateReader::new(slice, &section.reloc)
        });
        // SAFETY: erase lifetime. This is fine as `dwarf` will be dropped before `dwarf_sections`.
        let dwarf_transmute = unsafe {
            std::mem::transmute::<
                Dwarf<ReaderTy<'_, 'file, 'data>>,
                Dwarf<ReaderTy<'_, 'file, 'data>>,
            >(dwarf)
        };

        Ok(Self {
            dwarf: dwarf_transmute,
            dwarf_sections,
        })
    }

    /// Obtain the linkage name of a subprogram or inlined subroutine.
    fn linkage_name(
        &self,
        unit: &Unit<ReaderTy<'_, 'file, 'data>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_, 'file, 'data>>,
    ) -> Result<String, Error> {
        let mut attrs = die.attrs();
        let mut name = None;
        let mut deleg = None;

        while let Some(attr) = attrs.next()? {
            match attr.name() {
                gimli::DW_AT_linkage_name => {
                    return Ok(self
                        .dwarf
                        .attr_string(unit, attr.value())?
                        .to_string()?
                        .into_owned());
                }

                gimli::DW_AT_name => {
                    name = Some(
                        self.dwarf
                            .attr_string(unit, attr.value())?
                            .to_string()?
                            .into_owned(),
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
        unit: &Unit<ReaderTy<'_, 'file, 'data>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_, 'file, 'data>>,
    ) -> Result<(SectionIndex, Vec<Range<i64>>), Error> {
        let mut ranges = Vec::new();

        let mut attrs = die.attrs();
        while let Some(attr) = attrs.next()? {
            match attr.name() {
                gimli::DW_AT_low_pc => {
                    let Some(low_pc) = self.dwarf.attr_address(&unit, attr.value())? else {
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

                    let offset = self.dwarf.ranges_offset(unit, offset)?;
                    let mut range = self.dwarf.ranges(unit, offset)?;
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

        let encoded_section = decode_address(ranges[0].0).0;

        let ranges = ranges
            .into_iter()
            .map(|(begin, len)| {
                let (sec, begin) = decode_address(begin);
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
        unit: &Unit<ReaderTy<'_, 'file, 'data>>,
        die: &DebuggingInformationEntry<'_, '_, ReaderTy<'_, 'file, 'data>>,
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
        unit: &Unit<ReaderTy<'_, 'file, 'data>>,
        line: &LineProgramHeader<ReaderTy<'_, 'file, 'data>>,
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

            path.push(
                self.dwarf
                    .attr_string(&unit, directory)?
                    .to_string()?
                    .as_ref(),
            );
        }

        path.push(
            self.dwarf
                .attr_string(&unit, file.path_name())?
                .to_string()?
                .as_ref(),
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
                    let (encoded_section, encoded_offset) = decode_address(encoded_address);

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
