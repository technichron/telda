use std::{path::PathBuf, collections::{HashMap, HashSet, VecDeque, BTreeMap}, process::ExitCode};

use clap::{Parser, ArgGroup};
use telda2::{source::Format, aalv::{obj::{Object, GlobalSymbols, InternalSymbols, SymbolReferenceTable}, Segment}, disassemble::{DisassembledInstruction, disassemble_instruction}};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(group(
            ArgGroup::new("show")
                .required(true)
                .multiple(true)
                .args(["disassemble", "show_symbols"]),
        ))]
struct Cli {
    /// Input telda object file
    input_file: PathBuf,

    /// Disassemble symbols in .mem. If -D is not set, disassembles from all global symbols
    #[arg(short, long, group = "show")]
    disassemble: bool,
    /// If disassembling, sets the symbols to start disassembling from seperated by commas
    #[arg(short = 'D', long, requires = "disassemble", value_name = "SYMBOLS")]
    disassemble_from: Option<String>,
    
    /// Whether to show the symbol table segments
    #[arg(short = 't', long = "syms", group = "show")]
    show_symbols: bool,

    /// Shows relocations in disassembly
    #[arg(short = 'R', long, requires = "disassemble")]
    show_relocations: bool,
}

fn main() -> ExitCode {
    let Cli {
        input_file,
        disassemble,
        disassemble_from: dissasemble_from,
        show_symbols,
        show_relocations,
    } = Cli::parse();

    let obj = Object::from_file(&input_file).unwrap();

    if show_symbols {
        symbols(&obj);
    }
    if disassemble {
        disassembly(&obj, dissasemble_from, show_relocations);
    }

    ExitCode::SUCCESS
}

fn symbols(obj: &Object) {
    let mut defined_symbols = HashSet::new();
    let mut undefined_symbols = HashMap::new();
    
    if let Some(v) = &obj.global_symbols {
        println!("{}:", GlobalSymbols::NAME);
        for (l, a) in &v.0 {
            println!("    {l} = 0x{a:02x}");
            defined_symbols.insert(l);
        }
        println!();
    }
    if let Some(v) = &obj.internal_symbols {
        println!("{}:", InternalSymbols::NAME);
        for (l, a) in &v.0 {
            println!("    {l} = 0x{a:02x}");
            defined_symbols.insert(l);
        }
        println!();
    }
    if let Some(v) = &obj.symbol_reference_table {
        println!("{}:", SymbolReferenceTable::NAME);
        for (f, l, a) in &v.0 {
            print!("    ");
            match f {
                Format::Absolute => print!("A"),
                Format::Big => print!("B"),
            }
            println!(" {l} @ 0x{a:02x}");
            if !defined_symbols.contains(&l) {
                *undefined_symbols.entry(l).or_insert(0u64) += 1;
            }
        }
        println!();
    }

    if !undefined_symbols.is_empty() {
        println!("undefined symbols:");
        for (s, num) in undefined_symbols {
            let pl = if num == 1 { "" } else { "s" };
            println!("    {s} with {num} reference{pl}");
        }
        println!();
    }
}

fn disassembly(obj: &Object, start_symbol: Option<String>, show_relocations: bool) {
    let symbols: VecDeque<_>;
    if let Some(start_symbol) = start_symbol.as_ref() {
        symbols = start_symbol.split(',').map(|s| s.trim()).collect();
    } else {
        symbols = obj.global_symbols
            .as_ref()
            .map(|is| is.0.iter())
            .into_iter()
            .flatten()
            .map(|(s, _)| &**s)
            .collect();
    }

    println!(".mem:");
    let binary_code;
    let mut labels = HashMap::new();
    let mut pos_to_labels = HashMap::new();
    {
        binary_code = &*obj.mem.as_ref().unwrap().mem;

        let iter = obj.internal_symbols.as_ref()
            .map(|is| is.0.iter())
            .into_iter()
            .flatten()
            .chain(obj.global_symbols.as_ref().map(|is| is.0.iter()).into_iter().flatten());

        for &(ref label, position) in iter {
            labels.insert(label.clone(), position);
            pos_to_labels.insert(position, label.clone());
        }
    }

    let mut relocs = BTreeMap::new();
    if show_relocations {
        for &(_, ref sym, loc) in obj.symbol_reference_table.as_ref().map(|s| s.0.iter()).into_iter().flatten() {
            relocs.insert(loc, &**sym);
        }
    }

    let mut printed_labels = HashSet::new();
    let mut labels_to_print = symbols;

    while let Some(label_to_print) = labels_to_print.pop_front() {
        // Printed labels can end up in the queue
        if printed_labels.contains(&label_to_print) {
            continue;
        }

        println!("<{label_to_print}>:");
        printed_labels.insert(label_to_print);

        let mut location = labels[label_to_print];

        'labelled_block: loop {
            let DisassembledInstruction { annotated_source, ends_block, nesting_difference: _, next_instruction_location }
                = disassemble_instruction(location, &binary_code, |p| {
                    let l = pos_to_labels.get(&p).map(|s| &**s);
                    if let Some(l) = l {
                        if !printed_labels.contains(&l) {
                            labels_to_print.push_back(l);
                        }
                    }
                    l
                });

            if show_relocations {
                for (&loc, &sym) in relocs.range(location..next_instruction_location) {
                    println!("    RELOC: {sym} @ 0x{loc:02x}");
                }
            }
            println!("{}", annotated_source);
            if ends_block {
                break 'labelled_block;
            }
            if let Some(lbl) = pos_to_labels.get(&next_instruction_location) {
                if printed_labels.insert(lbl) {
                    // Was not printed before
                    println!("<{lbl}>:");
                } else {
                    // Was printed before => end block
                    println!("<{lbl}> ...");
                    break 'labelled_block;
                }
            }
            location = next_instruction_location;
        }
        println!();
    }
}