fn main() {
    if let Err(error) = psd2ugui::run() {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
