mod auto_scroll;
mod capture;
mod cli;
mod constants;
mod control_socket;
mod opencv_compat;
mod output;
mod overlay;
mod region_overlay;
mod selection;
mod session;
mod stitch;
mod types;
mod ui;

use anyhow::Result;

use crate::cli::Args;

/// Program entrypoint.
fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse_args();
    let result = session::run(args);
    if let Err(error) = &result {
        let _ = std::process::Command::new("notify-send")
            .args(["Scrolling capture stopped", &format!("{error:#}")])
            .status();
    }
    result
}
