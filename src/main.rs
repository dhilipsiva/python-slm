fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    std::process::exit(rust_llm_pretrain::commands::entry(std::env::args_os()));
}
