use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use prisma_orchestrator::pe_loader::{parse, parse_imports, ImportSymbol};

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: pe_import_inventory <windows.exe-or-dll>")?;
    let file = fs::read(&path)?;
    let image = parse(&file)?;
    let imports = parse_imports(&image, &file)?;
    let symbol_count: usize = imports.iter().map(|item| item.symbols.len()).sum();

    println!(
        "PE|path={}|machine={:?}|format={}|entry={:#x}|image_size={}|dlls={}|symbols={}",
        path.display(),
        image.machine,
        if image.pe32_plus { "PE32+" } else { "PE32" },
        image.image_base + u64::from(image.entry_point_rva),
        image.size_of_image,
        imports.len(),
        symbol_count
    );
    for item in imports {
        println!("DLL|name={}|symbols={}", item.dll, item.symbols.len());
        for symbol in item.symbols {
            match symbol {
                ImportSymbol::Name(name) => println!("IMPORT|dll={}|name={name}", item.dll),
                ImportSymbol::Ordinal(ordinal) => {
                    println!("IMPORT|dll={}|ordinal={ordinal}", item.dll);
                }
            }
        }
    }
    Ok(())
}
