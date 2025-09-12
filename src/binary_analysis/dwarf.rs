use std::borrow::Cow;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::Arc;

use gimli::{Dwarf, EndianSlice, LineRow, Reader, RelocateReader};
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

type DwarfTy<'a, 'file, 'data> =
    Dwarf<RelocateReader<EndianSlice<'a, gimli::LittleEndian>, &'a SectionRelocate<'file, 'data>>>;

pub struct DwarfLoader<'file, 'data> {
    // This is actually `DwarfTy<'dwarf_sections, 'file, 'data>`.
    dwarf: DwarfTy<'file, 'file, 'data>,
    #[allow(unused)]
    dwarf_sections: Arc<gimli::DwarfSections<SectionWithReloc<'file, 'data>>>,
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
            std::mem::transmute::<DwarfTy<'_, 'file, 'data>, DwarfTy<'_, 'file, 'data>>(dwarf)
        };

        Ok(Self {
            dwarf: dwarf_transmute,
            dwarf_sections,
        })
    }

    pub fn locate<'tcx>(
        &self,
        section_index: SectionIndex,
        offset: u64,
    ) -> Result<Option<(PathBuf, u32, u32)>, Error> {
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

                    if let Some((prev_addr, prev_row)) = prev {
                        if (prev_addr..encoded_offset).contains(&(offset as i64)) {
                            // Found a line info that covers this!
                            let file = rows.header().file(prev_row.file_index()).ok_or(
                                Error::UnexpectedDwarf("debug_lines referenced non-existent file"),
                            )?;

                            let mut path = PathBuf::new();

                            if file.directory_index() != 0 {
                                let directory =
                                    file.directory(rows.header()).ok_or(Error::UnexpectedDwarf(
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

                            let line = prev_row.line().map_or(0, NonZero::get) as u32;
                            let column = match prev_row.column() {
                                gimli::ColumnType::LeftEdge => 0,
                                gimli::ColumnType::Column(v) => v.get() as u32,
                            };
                            return Ok(Some((path, line, column)));
                        }
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
