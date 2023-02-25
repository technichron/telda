use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{self, Seek, Write},
    num::ParseIntError,
    // os::unix::prelude::PermissionsExt,
    path::PathBuf,
    process::ExitCode,
};

use clap::Parser;
use collect_result::CollectResult;
use telda2::{
    aalv::obj::{
        Entry, Object, RelocationEntry, RelocationTable, SegmentType, SymbolDefinition, SymbolTable,
    },
    align, SEGMENT_ALIGNMENT,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Input telda object files
    input_files: Vec<PathBuf>,

    /// Sets the output path, otherwise a.out is used
    #[arg(short, long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Defines an entry-point and makes this output an executable binary
    ///
    /// Can be either a hexadecimal address prefixed by 0x or a symbol
    ///
    /// Without this, the first entry-point from the given input object files is used.
    ///
    /// This means if multiple objects with an entry point are given,
    /// only the first one will be used for the entry point of the output of this.
    #[arg(short = 'E', long, requires = "executable")]
    set_entry: Option<String>,

    /// Erase internal symbols
    #[arg(short = 'S', long)]
    strip_internal: bool,

    /// Makes the output file an executable binary which
    /// disallows undefined references
    ///
    /// Errors if no entry-point is defined in input files or with -E
    #[arg(short = 'e', long)]
    executable: bool,
}

fn main() -> ExitCode {
    match tl_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            match e {
                Error::Io(e) => eprintln!("io error: {e}"),
                Error::InvalidEntryPointFormat(e) => eprintln!("invalid entry point format: {e}"),
                Error::NoEntryPoint => eprintln!("No entry point was defined, cannot make executable. Perhaps use -E to set one?"),
                Error::ReferenceToNonExistantSegment => eprintln!("reference to a segment that was not defined"),
                Error::ObjectFailure => (),
            }

            ExitCode::FAILURE
        }
    }
}

enum Error {
    Io(io::Error),
    InvalidEntryPointFormat(ParseIntError),
    ObjectFailure,
    NoEntryPoint,
    ReferenceToNonExistantSegment,
}

fn tl_main() -> Result<(), Error> {
    let Cli {
        input_files,
        out,
        set_entry,
        strip_internal,
        executable,
    } = Cli::parse();

    let objects: Vec<_> = input_files
        .into_iter()
        .map(|p| Object::from_file(&p).map(|o| (p, o)))
        .collect_result()
        .map_err(Error::Io)?;

    let mut segs_out = BTreeMap::new();

    {
        let mut lengths = BTreeMap::new();

        for (_, obj) in &objects {
            for (&stype, &(_start, ref v)) in &obj.segs {
                *lengths.entry(stype).or_insert(0) += v.len() as u16;
            }
        }
        let mut last_end = lengths.remove(&SegmentType::Zero).unwrap_or(0);
        last_end = last_end.max(SEGMENT_ALIGNMENT);

        for (st, size) in lengths {
            let start = align(last_end, SEGMENT_ALIGNMENT);
            segs_out.insert(st, (start, Vec::with_capacity(size as usize)));
            last_end = start + size;
        }
    }

    let mut segs: BTreeMap<_, _> = segs_out
        .iter_mut()
        .map(|(&st, &mut (start, ref mut bytes))| (st, (start, bytes)))
        .collect();

    let out = out.unwrap_or_else(|| PathBuf::from("a.to"));

    let mut global_symbols = HashMap::new();
    let mut symbols_out = Vec::new();
    let mut reloc_out = Vec::new();
    let mut undefined_references = Vec::new();

    let mut entry_point = None;

    let mut failure = false;

    for (input_file, mut obj) in objects {
        entry_point = entry_point.or_else(|| {
            obj.entry
                .map(|Entry(st, ep)| Entry(st, ep - obj.segs[&st].0 + segs[&st].0))
        });

        let mut file_symbol_to_out_symbol = Vec::new();
        let reloc;
        {
            for mut symdef in obj.symbols.into_iter() {
                let next_id = symbols_out.len();
                let mut id_in_fstos = None;

                symdef.location -= obj.segs.get(&symdef.segment_type).map(|s| s.0).unwrap_or(0);
                symdef.location += segs.get(&symdef.segment_type).map(|s| s.0).unwrap_or(0);

                if symdef.is_global {
                    match global_symbols.get(&symdef.name) {
                        None => {
                            global_symbols.insert(symdef.name.clone(), next_id);
                        }
                        Some(&id) => {
                            let cur_symdef: &mut SymbolDefinition = &mut symbols_out[id];

                            if let SegmentType::Unknown = symdef.segment_type {
                            } else if let SegmentType::Unknown = cur_symdef.segment_type {
                                *cur_symdef = symdef.clone();
                            } else {
                                eprintln!("global symbol {} defined in {} but was already defined in a previous file at location 0x{:02x} in {}",
                                    symdef.name,
                                    input_file.display(),
                                    symdef.location,
                                    symdef.segment_type,
                                );
                                failure = true;
                            }

                            id_in_fstos = Some(id);
                        }
                    }
                } else if strip_internal {
                    symdef.name = "".into();
                }

                let id;
                if let Some(id_in_fstos) = id_in_fstos {
                    id = id_in_fstos;
                } else {
                    symbols_out.push(symdef);
                    id = next_id;
                }
                file_symbol_to_out_symbol.push(id);
            }
            reloc = obj.relocation_table.0;
        }

        for RelocationEntry {
            reference_location,
            reference_segment,
            symbol_index,
        } in reloc
        {
            let symbol_index = file_symbol_to_out_symbol[symbol_index as usize];

            let location_in_file = reference_location - obj.segs[&reference_segment].0;
            let reference_location = location_in_file + segs[&reference_segment].0;

            let bytes = &mut obj
                .segs
                .get_mut(&reference_segment)
                .ok_or(Error::ReferenceToNonExistantSegment)?
                .1;

            let symdef = &symbols_out[symbol_index];
            let undefined = matches!(symdef.segment_type, SegmentType::Unknown);

            bytes[location_in_file as usize..location_in_file as usize + 2]
                .copy_from_slice(&symdef.location.to_le_bytes());

            let entry = RelocationEntry {
                reference_location,
                reference_segment,
                symbol_index: symbol_index as u16,
            };

            reloc_out.push(entry);
            if undefined {
                undefined_references.push(entry);
            }
        }

        for (t, (_, bytes)) in obj.segs {
            let seg = segs.get_mut(&t).expect("segment guaranteed to exist");
            seg.0 += bytes.len() as u16;
            seg.1.extend(bytes);
        }
    }
    drop(segs);

    for RelocationEntry {
        reference_segment,
        reference_location,
        symbol_index,
    } in undefined_references
    {
        let symdef = &symbols_out[symbol_index as usize];
        if let SegmentType::Unknown = symdef.segment_type {
            if executable {
                eprintln!(
                    "undefined reference to {} at 0x{:02x}",
                    symdef.name, symdef.location
                );
                failure = true;
            }
            continue;
        };

        let seg = segs_out
            .get_mut(&reference_segment)
            .expect("would have been caught earlier");
        let index = (reference_location - seg.0) as usize;
        seg.1[index..index + 2].copy_from_slice(&symdef.location.to_le_bytes());
    }

    if let Some(entry) = set_entry {
        entry_point = Some({
            if let Some(entry) = entry.strip_prefix("0x") {
                Entry(
                    SegmentType::Zero,
                    u16::from_str_radix(entry, 16).map_err(Error::InvalidEntryPointFormat)?,
                )
            } else if let Some(&pos) = global_symbols.get(&*entry) {
                let sym = &symbols_out[pos];
                Entry(sym.segment_type, sym.location)
            } else {
                eprintln!("Start symbol {entry} was not found. Perhaps it is not global?");
                eprintln!("Aborting linking");
                failure = true;
                Entry(SegmentType::Unknown, 0xffff)
            }
        });
    };

    if failure {
        return Err(Error::ObjectFailure);
    }

    let obj = Object {
        segs: segs_out,
        entry: entry_point,
        symbols: SymbolTable(symbols_out),
        relocation_table: RelocationTable(reloc_out),
        ..Object::default()
    };

    if executable {
        if obj.entry.is_none() {
            return Err(Error::NoEntryPoint);
        }

        let mut obj = obj;
        {
            let mut file = File::create(&out).map_err(Error::Io)?;
            writeln!(file, "#!/bin/env t").map_err(Error::Io)?;

            obj.file_offset = file.stream_position().map_err(Error::Io)?;
        }

        obj.write_to_file(&out).map_err(Error::Io)?;

        let mut perms = fs::metadata(&out).map_err(Error::Io)?.permissions();
        // perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&out, perms).map_err(Error::Io)?;
    } else {
        obj.write_to_file(out).map_err(Error::Io)?;
    }

    Ok(())
}