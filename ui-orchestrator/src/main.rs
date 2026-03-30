fn main() {
    if let Err(error) = ui_orchestrator::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
