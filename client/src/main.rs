//! `velocide`: the game client binary. See `client::cli::USAGE` for the options.

use client::app::{Launch, run};
use client::cli::{self, Cli};
use client::settings::Settings;
use client::update;

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
    // An update on Windows leaves the replaced executable as `<exe>.old`.
    if let Ok(exe) = std::env::current_exe() {
        update::install::remove_old(&exe);
    }
    let settings_path = Settings::default_path();
    let settings = Settings::load(&settings_path);
    log::info!(
        "{} {} (settings: {})",
        shared::GAME_NAME,
        shared::build_info::label(),
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
    // UPDATE NOW finished: the window is closed and settings are saved, start the new game.
    if let Some(exe) = update::take_relaunch() {
        log::info!("restarting into the update: {}", exe.display());
        if let Err(e) = update::relaunch(&exe) {
            eprintln!("velocide: {e:#}");
            std::process::exit(1);
        }
    }
}
