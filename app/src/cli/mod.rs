//! Command-line interface: text in, a validated structure out. No side effects.

use clap::{Parser, ValueEnum};
use wgpu::Backends;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "world",
    version,
    about = "World: a micro-voxel destruction sandbox"
)]
pub struct Cli {
    /// Graphics backend. `auto` picks the best adapter the OS offers.
    #[arg(
        long,
        env = "WGPU_BACKEND",
        value_enum,
        ignore_case = true,
        default_value_t = BackendChoice::Auto
    )]
    pub backend: BackendChoice,

    /// Approximate the 2019 MacBook Pro floor: tier B, 720p internal, 1 GiB brick pool, 4 workers.
    #[arg(long)]
    pub mac_floor: bool,

    /// Print every adapter found with its tier verdict, then exit.
    #[arg(long)]
    pub list_adapters: bool,

    /// Developer: exit cleanly after this many frames (for automated runs).
    #[arg(long, value_name = "FRAMES")]
    pub exit_after_frames: Option<u32>,

    /// Developer: save a screenshot of the window to this PNG path, shortly
    /// before `--exit-after-frames` ends the run.
    #[arg(long, value_name = "PATH", requires = "exit_after_frames")]
    pub screenshot: Option<std::path::PathBuf>,

    /// Developer: write every frame's time in milliseconds, one per line, to
    /// this path when `--exit-after-frames` ends the run.
    #[arg(long, value_name = "PATH", requires = "exit_after_frames")]
    pub frame_times: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BackendChoice {
    Auto,
    #[value(alias = "d3d12")]
    Dx12,
    Vulkan,
    Metal,
}

impl BackendChoice {
    /// The backend set to probe, or an error naming a backend this OS cannot offer.
    pub fn to_backends(self) -> Result<Backends, String> {
        let on_windows = cfg!(target_os = "windows");
        let on_macos = cfg!(target_os = "macos");
        match self {
            Self::Auto if on_windows => Ok(Backends::DX12 | Backends::VULKAN),
            Self::Auto if on_macos => Ok(Backends::METAL),
            Self::Auto => Ok(Backends::VULKAN),
            Self::Dx12 if on_windows => Ok(Backends::DX12),
            Self::Vulkan if !on_macos => Ok(Backends::VULKAN),
            Self::Metal if on_macos => Ok(Backends::METAL),
            Self::Dx12 => Err("--backend dx12 is only available on Windows".into()),
            Self::Vulkan => Err("--backend vulkan is not available on macOS".into()),
            Self::Metal => Err("--backend metal is only available on macOS".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let cli = Cli::try_parse_from(["world", "--backend", "vulkan", "--mac-floor"]).unwrap();
        assert_eq!(cli.backend, BackendChoice::Vulkan);
        assert!(cli.mac_floor);
    }

    #[test]
    fn rejects_unknown_backend_naming_the_flag() {
        let err = Cli::try_parse_from(["world", "--backend", "banana"]).unwrap_err();
        assert!(err.to_string().contains("--backend"));
    }

    #[test]
    fn frame_times_need_a_fixed_length_run() {
        assert!(Cli::try_parse_from(["world", "--frame-times", "t.txt"]).is_err());
        let cli = Cli::try_parse_from([
            "world",
            "--exit-after-frames",
            "60",
            "--frame-times",
            "t.txt",
        ])
        .unwrap();
        assert_eq!(
            cli.frame_times.as_deref(),
            Some(std::path::Path::new("t.txt"))
        );
    }

    #[test]
    fn auto_always_resolves_on_this_os() {
        assert!(BackendChoice::Auto.to_backends().is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn metal_is_rejected_on_windows() {
        assert!(BackendChoice::Metal.to_backends().is_err());
    }
}
