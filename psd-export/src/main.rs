fn main() {
    if let Err(error) = psd_export::run() {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
