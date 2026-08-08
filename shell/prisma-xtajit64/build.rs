use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let header = manifest_dir
        .join("../..")
        .join("third_party/wine/dlls/ntdll/ntsyscalls.h");
    println!("cargo:rerun-if-changed={}", header.display());

    let source = fs::read_to_string(&header).unwrap_or_else(|error| {
        panic!(
            "cannot read Wine syscall table {}: {error}",
            header.display()
        )
    });
    let win64_start = source
        .find("#ifdef _WIN64")
        .expect("Wine Win64 syscall section");
    let section = &source[win64_start..];
    let table_start = section
        .find("#define ALL_SYSCALLS \\")
        .expect("Wine Win64 ALL_SYSCALLS table");
    let table = &section[table_start..];
    let table_end = table
        .find("\n#else")
        .expect("end of Wine Win64 syscall table");

    let mut entries = Vec::new();
    for line in table[..table_end].lines() {
        let Some(macro_start) = line.find("SYSCALL_ENTRY") else {
            continue;
        };
        let call = &line[macro_start..];
        let open = call.find('(').expect("syscall macro opening parenthesis");
        let close = call.rfind(')').expect("syscall macro closing parenthesis");
        let fields = call[open + 1..close]
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>();
        assert_eq!(fields.len(), 3, "unexpected Wine syscall row: {line}");
        let id = u16::from_str_radix(
            fields[0]
                .strip_prefix("0x")
                .expect("hexadecimal Wine syscall id"),
            16,
        )
        .expect("valid Wine syscall id");
        let name = fields[1];
        assert!(
            name.starts_with("Nt")
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "invalid Wine syscall name: {name}"
        );
        let argument_bytes = fields[2]
            .parse::<u16>()
            .expect("valid Wine syscall argument byte count");
        assert_eq!(
            argument_bytes % 8,
            0,
            "Win64 arguments are eight-byte slots"
        );
        entries.push((id, name.to_owned(), argument_bytes));
    }

    entries.sort_by_key(|entry| entry.0);
    assert!(!entries.is_empty(), "Wine Win64 syscall table is empty");
    for (expected, (actual, _, _)) in entries.iter().enumerate() {
        assert_eq!(
            usize::from(*actual),
            expected,
            "Wine syscall ids must be dense"
        );
    }

    let mut generated = String::from(
        "// Generated from Wine 11.14 dlls/ntdll/ntsyscalls.h; do not edit.\n\
         const WINE_SYSCALLS: &[WineSyscallEntry] = &[\n",
    );
    for (_, name, argument_bytes) in entries {
        writeln!(
            generated,
            "    WineSyscallEntry {{ name: b\"{name}\\0\", argument_bytes: {argument_bytes} }},"
        )
        .expect("write generated syscall row");
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo OUT_DIR"));
    fs::write(out_dir.join("wine_syscalls.rs"), generated).expect("write Wine syscall table");
}
