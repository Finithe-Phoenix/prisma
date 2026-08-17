fn main() {
    let report = prisma_android::run_execution_probe();
    println!("{report}");
    if !report.starts_with("REAL|") {
        std::process::exit(1);
    }
}
