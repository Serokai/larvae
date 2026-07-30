//! `coldluau self <command>`, manage the coldluau installation itself

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use semver::Version;

use crate::net::{github, http};
use crate::sys::paths;
use crate::ui;

/// GitHub repository releases are published to
const REPO: &str = "coldluau/cli";

#[derive(Subcommand)]
pub enum SelfCommand {
    /// Install coldluau to ~/.coldluau/bin
    Install,

    /// Update coldluau to the latest release
    Update,
    
    /// Remove coldluau from this machine
    Uninstall,
}

pub fn run(cmd: SelfCommand) -> Result<ExitCode> {
    match cmd {
        SelfCommand::Install => install(),
        SelfCommand::Update => update(),
        SelfCommand::Uninstall => uninstall(),
    }
}

fn install() -> Result<ExitCode> {
    let me = std::env::current_exe().context("cannot locate the running executable")?;
    let bin_dir = paths::bin_dir()?;
    let target = paths::installed_exe()?;

    if paths::same_file(&me, &target) {
        ui::print_success(&format!(
            "coldluau is already installed at {}",
            target.display()
        ));
    } else {
        std::fs::create_dir_all(&bin_dir)
            .with_context(|| format!("failed to create {}", bin_dir.display()))?;
        std::fs::copy(&me, &target)
            .with_context(|| format!("failed to copy to {}", target.display()))?;
        ui::print_success(&format!("Installed coldluau to {}", target.display()));
    }

    print_path_instructions(&bin_dir);
    Ok(ExitCode::SUCCESS)
}

fn update() -> Result<ExitCode> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    eprintln!("Checking for updates (currently v{current})");

    let release = github::latest_release(REPO)
        .with_context(|| format!("failed to query releases for {REPO}"))?;
    
    let latest = Version::parse(release.tag_name.trim_start_matches('v'))
        .with_context(|| format!("release tag {:?} is not a version", release.tag_name))?;
    
    if latest <= current {
        ui::print_success("coldluau is already up to date");
        return Ok(ExitCode::SUCCESS);
    }

    // Release assets are named coldluau-{os}-{arch}[.exe] from std::env::consts
    let asset_name = format!(
        "coldluau-{}-{}{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::EXE_SUFFIX
    );

    let Some(asset) = release.assets.iter().find(|a| a.name == asset_name) else {
        bail!("release v{latest} has no asset named {asset_name}");
    };

    eprintln!("Downloading {} v{latest}", asset.name);

    let bytes = http::get_bytes(&asset.browser_download_url)?;
    let staged = std::env::temp_dir().join(&asset_name);
    
    std::fs::write(&staged, &bytes)
        .with_context(|| format!("failed to stage {}", staged.display()))?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }

    self_replace::self_replace(&staged).context("failed to replace the running executable")?;
    let _ = std::fs::remove_file(&staged);

    ui::print_success(&format!("Updated coldluau v{current} -> v{latest}"));
    Ok(ExitCode::SUCCESS)
}

fn uninstall() -> Result<ExitCode> {
    let dir = paths::coldluau_dir()?;
    
    if !dir.exists() {
        bail!("coldluau is not installed at {}", dir.display());
    }

    if !ui::confirm(&format!("Remove {}?", dir.display()), false) {
        eprintln!("Aborted.");
        return Ok(ExitCode::SUCCESS);
    }

    // a binary inside ~/.coldluau deletes itself first so the dir can go
    let me = std::env::current_exe()?;

    if me
        .canonicalize()
        .map(|p| p.starts_with(&dir))
        .unwrap_or(false)
    {
        self_replace::self_delete_outside_path(&dir)
            .context("failed to remove the running executable")?;
    }

    std::fs::remove_dir_all(&dir).with_context(|| format!("failed to remove {}", dir.display()))?;
    ui::print_success(&format!("Removed {}", dir.display()));
    
    let bin = paths::bin_dir()?;
    
    eprintln!("You can now drop {} from your PATH.", bin.display());
    Ok(ExitCode::SUCCESS)
}

fn print_path_instructions(bin_dir: &std::path::Path) {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|entry| entry == bin_dir))
        .unwrap_or(false);
    
    if on_path {
        return;
    }

    if cfg!(windows) {
        eprintln!(
            "Add {} to your PATH (Settings > Environment Variables), then open a new terminal.",
            bin_dir.display()
        );
    } else {
        eprintln!("Add this to your shell profile, then open a new terminal:");
        eprintln!("  export PATH=\"{}:$PATH\"", bin_dir.display());
    }
}
