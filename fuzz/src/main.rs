//! Deterministic bounded replay; this is not a substitute for a sustained fuzz campaign.
use std::{io::Write, path::PathBuf};
use taypeer_parser_fuzz::{Parser, seed_inputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let destination = match arguments.as_slice() {
        [] => None,
        [option, path] if option == "--write-seeds" => Some(PathBuf::from(path)),
        _ => return Err("usage: corpus-smoke [--write-seeds NEW_DIRECTORY]".into()),
    };
    if let Some(path) = &destination {
        std::fs::create_dir(path)?;
        for target in [Parser::Document, Parser::Storage, Parser::Trust] {
            std::fs::create_dir(path.join(target.name()))?;
        }
    }
    let seeds = seed_inputs();
    let mut cases = 0;
    for (index, (parser, name, bytes)) in seeds.iter().enumerate() {
        if let Some(path) = &destination {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join(parser.name()).join(format!("{index:02}-{name}")))?;
            file.write_all(bytes)?;
        }
        parser.run(bytes);
        parser.run(&[]);
        parser.run(&bytes[..bytes.len() / 2]);
        parser.run(&bytes[..bytes.len().saturating_sub(1)]);
        cases += 4;
        for position in [0, bytes.len() / 2, bytes.len().saturating_sub(1)] {
            if !bytes.is_empty() {
                let mut corrupted = bytes.clone();
                corrupted[position] ^= 0x80;
                parser.run(&corrupted);
                cases += 1;
            }
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        parser.run(&trailing);
        cases += 1;
    }
    println!(
        "Parser smoke passed: {} PUBLIC seeds, {cases} deterministic cases",
        seeds.len()
    );
    Ok(())
}
