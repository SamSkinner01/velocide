//! `velocide`: the game client binary. See `client::cli::USAGE` for the options.

use client::app::{Launch, run};
use client::cli::{self, Cli};
use client::settings::Settings;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(Cli::Run(options)) => *options,
        Ok(Cli::Help) => {
            println!("{}", cli::USAGE);
            return;
        }
        Err(e) => {
            eprintln!("velocide: {e}");
            std::process::exit(2);
        }
    };
    let settings_path = Settings::default_path();
    let settings = Settings::load(&settings_path);
    log::info!(
        "{} (settings: {})",
        shared::GAME_NAME,
        settings_path.display()
    );
    if let Err(e) = run(Launch {
        options,
        settings,
        settings_path,
    }) {
        eprintln!("velocide: {e:#}");
        std::process::exit(1);
    }
}
