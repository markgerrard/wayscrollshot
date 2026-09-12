use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::constants::PREVIEW_MAX_WIDTH;

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum Algorithm {
    /// Column sampling (fast, good for most cases)
    ColSample,
    /// Template matching (slower, more accurate)
    Template,
    /// Edge detection (for transparent backgrounds)
    Edge,
    /// FAST corner + HNSW index (high accuracy, from snow-shot)
    Fast,
    /// OpenCV ORB + brute-force matching + affine RANSAC
    #[default]
    #[value(name = "opencv-orb")]
    OpenCvOrb,
}

#[derive(Parser, Debug, Clone)]
#[command(name = "wayscrollshot")]
#[command(about = "A scrolling screenshot tool for Wayland", long_about = None)]
pub struct Args {
    /// Output file path (default: ~/Pictures/wayscrollshot/wayscrollshot-<timestamp>.png).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Scroll automatically, saving when the page stops moving.
    #[arg(long)]
    pub auto_scroll: bool,

    /// Finish an existing capture, or start one if none is running.
    #[arg(long)]
    pub toggle: bool,

    /// Milliseconds of stable content required before accepting a frame.
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(100..=5000))]
    pub settle_ms: u64,

    /// Percentage of the crop height to advance per automatic scroll step.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(10..=60))]
    pub scroll_percent: u32,

    /// Preview width in pixels
    #[arg(short = 'w', long, default_value_t = PREVIEW_MAX_WIDTH)]
    pub preview_width: u32,

    /// Copy to clipboard instead of saving to file
    #[arg(short, long)]
    pub clipboard: bool,

    /// Hide live preview and save immediately without final review
    #[arg(long)]
    pub no_preview: bool,

    /// Hide the capture mask and outline
    #[arg(long)]
    pub no_border: bool,

    /// Stitching algorithm to use
    #[arg(short, long, value_enum, default_value_t = Algorithm::ColSample)]
    pub algorithm: Algorithm,

    /// Existing slurp geometry to use instead of selecting a region. Use '-' to read from stdin.
    #[arg(value_name = "REGION", num_args = 0..=2, allow_hyphen_values = true)]
    pub region: Vec<String>,
}

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }

    pub fn slurp_output(&self) -> Option<String> {
        if self.region.is_empty() {
            None
        } else {
            Some(self.region.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_quoted_slurp_region() {
        let args = Args::try_parse_from(["wayscrollshot", "10,20 300x400"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }

    #[test]
    fn parses_unquoted_slurp_region_parts() {
        let args = Args::try_parse_from(["wayscrollshot", "10,20", "300x400"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }

    #[test]
    fn parses_stdin_region_marker() {
        let args = Args::try_parse_from(["wayscrollshot", "-"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("-"));
    }

    #[test]
    fn parses_negative_slurp_coordinates() {
        let args = Args::try_parse_from(["wayscrollshot", "-1920,-10", "800x600"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("-1920,-10 800x600"));
    }

    #[test]
    fn keeps_existing_flags_with_region() {
        let args = Args::try_parse_from(["wayscrollshot", "-c", "10,20", "300x400"]).unwrap();

        assert!(args.clipboard);
        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }
}
