//! A simple example of parsing `.debug_line`.

use object::{Object, ObjectSection};
use std::{
    borrow,
    collections::HashMap,
    env, fs,
    path::{self, PathBuf},
};

fn main() {
    for path in env::args().skip(1) {
        let file = fs::File::open(&path).unwrap();
        let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
        let object = object::File::parse(&*mmap).unwrap();
        let endian = if object.is_little_endian() {
            gimli::RunTimeEndian::Little
        } else {
            gimli::RunTimeEndian::Big
        };
        dump_file(&path, &object, endian).unwrap();
    }
}

fn dump_file(
    exe: &str,
    object: &object::File,
    endian: gimli::RunTimeEndian,
) -> Result<(), gimli::Error> {
    // Load a section and return as `Cow<[u8]>`.
    let load_section = |id: gimli::SectionId| -> Result<borrow::Cow<[u8]>, gimli::Error> {
        match object.section_by_name(id.name()) {
            Some(ref section) => Ok(section
                .uncompressed_data()
                .unwrap_or(borrow::Cow::Borrowed(&[][..]))),
            None => Ok(borrow::Cow::Borrowed(&[][..])),
        }
    };

    // Load all of the sections.
    let dwarf_cow = gimli::Dwarf::load(&load_section)?;

    // Borrow a `Cow<[u8]>` to create an `EndianSlice`.
    let borrow_section: &dyn for<'a> Fn(
        &'a borrow::Cow<[u8]>,
    ) -> gimli::EndianSlice<'a, gimli::RunTimeEndian> =
        &|section| gimli::EndianSlice::new(&*section, endian);

    // Create `EndianSlice`s for all of the sections.
    let dwarf = dwarf_cow.borrow(&borrow_section);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    struct Line(u64);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    struct Column(u64);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    struct Occurrences(u64);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    struct Address(u64);

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    struct LineInfo {
        addresses: Vec<Address>,
        columns: HashMap<Column, Occurrences>,
    };

    let mut bytes_on_line: HashMap<PathBuf, HashMap<Line, LineInfo>> = HashMap::new();

    // Iterate over the compilation units.
    let mut iter = dwarf.units();
    while let Some(header) = iter.next()? {
        // println!(
        //     "Line number info for unit at <.debug_info+0x{:x}>",
        //     header.offset().as_debug_info_offset().unwrap().0
        // );
        let unit = dwarf.unit(header)?;

        // Get the line program for the compilation unit.
        if let Some(program) = unit.line_program.clone() {
            let comp_dir = if let Some(ref dir) = unit.comp_dir {
                path::PathBuf::from(dir.to_string_lossy().into_owned())
            } else {
                path::PathBuf::new()
            };

            // Iterate over the line program rows.
            let mut rows = program.rows();
            while let Some((header, row)) = rows.next_row()? {
                if row.end_sequence() {
                    // End of sequence indicates a possible gap in addresses.
                    // println!("{:x} end-sequence", row.address());
                } else {
                    // Determine the path. Real applications should cache this for performance.
                    let mut path = path::PathBuf::new();
                    if let Some(file) = row.file(header) {
                        path = comp_dir.clone();

                        // The directory index 0 is defined to correspond to the compilation unit directory.
                        if file.directory_index() != 0 {
                            if let Some(dir) = file.directory(header) {
                                path.push(
                                    dwarf.attr_string(&unit, dir)?.to_string_lossy().as_ref(),
                                );
                            }
                        }

                        path.push(
                            dwarf
                                .attr_string(&unit, file.path_name())?
                                .to_string_lossy()
                                .as_ref(),
                        );
                    }

                    // Determine line/column. DWARF line/column is never 0, so we use that
                    // but other applications may want to display this differently.
                    let line = match row.line() {
                        Some(line) => line.get(),
                        None => 0,
                    };
                    let column = match row.column() {
                        gimli::ColumnType::LeftEdge => 0,
                        gimli::ColumnType::Column(column) => column.get(),
                    };

                    let lines = bytes_on_line.entry(path.clone()).or_default();
                    let LineInfo { addresses, columns } = lines.entry(Line(line)).or_default();
                    let occurrences = columns.entry(Column(column)).or_default();
                    occurrences.0 += 1;
                    addresses.push(Address(row.address()));

                    // println!("{:x} {}:{}:{}", row.address(), path.display(), line, column);
                }
            }
        }
    }

    for (path, lines) in &bytes_on_line {
        for (line, line_info) in lines {
            // println!(
            //     " File: {} Line: {}",
            //     path.display(),
            //     line.0,
            //     //total_accumulation
            // );

            let instructions = line_info
                .columns
                .values()
                .fold(0u64, |acc, occurrences| acc + occurrences.0);
            if instructions > 1 {
                println!(
                    "{}:{} instructions: {}       addr2line --demangle --functions --exe {} {}",
                    path.display(),
                    line.0,
                    instructions,
                    exe,
                    line_info
                        .addresses
                        .iter()
                        .map(|a| format!("{:#x}", a.0))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }

    Ok(())
}
