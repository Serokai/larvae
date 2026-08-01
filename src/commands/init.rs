//! `coldluau init`, scaffold a starter coldluau.toml

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::ui;

pub fn run(root: &Path) -> Result<ExitCode> {
    let path = root.join("coldluau.toml");

    if path.exists() {
        bail!("coldluau.toml already exists");
    }

    let has_project = root.join("default.project.json").exists();
    let project_note = if has_project {
        "# Mounts are auto-derived from default.project.json.\n"
    } else {
        "# No default.project.json found: for the roblox-string target you'll need\n# [requires.mounts] or a Rojo project file.\n"
    };

    let template = format!(
        "{schema}\n\
         # coldluau configuration - see coldluau.example.toml for every option.\n\
         {project_note}\n\
         [process]\n\
         input = \"src\"\n\
         output = \"dist\"\n\n\
         [requires]\n\
         target = \"roblox-string\"\n",
        schema = crate::commands::schema::directive()
    );
    std::fs::write(&path, template)?;
    ui::print_success(&format!("Wrote {}", crate::ui::rel(&path)));
    ensure_gitignore(root)?;

    Ok(ExitCode::SUCCESS)
}

/// Entries coldluau's outputs need ignored
const IGNORE_ENTRIES: [&str; 2] = [".coldluau/", "dist/"];

/// Keep the output dirs ignored, append to an existing .gitignore or offer to create one
fn ensure_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".gitignore");

    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let missing: Vec<&str> = IGNORE_ENTRIES
            .iter()
            .filter(|e| !ignores(&content, e))
            .copied()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }

        let mut out = content;

        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }

        for entry in &missing {
            out.push_str(entry);
            out.push('\n');
        }

        std::fs::write(&path, out)?;
        ui::print_success(&format!("Added {} to .gitignore", missing.join(" and ")));
    } else if ui::confirm(
        "No .gitignore found. Create one ignoring .coldluau/ and dist/?",
        true,
    ) {
        std::fs::write(&path, format!("{}\n", IGNORE_ENTRIES.join("\n")))?;
        ui::print_success(&format!("Created {}", crate::ui::rel(&path)));
    }

    Ok(())
}

/// True when the content already covers entry, slashes optional
fn ignores(content: &str, entry: &str) -> bool {
    let want = entry.trim_start_matches('/').trim_end_matches('/');
    content
        .lines()
        .any(|line| line.trim().trim_start_matches('/').trim_end_matches('/') == want)
}

#[cfg(test)]
mod tests {
    use super::ignores;

    #[test]
    fn gitignore_matching() {
        assert!(ignores("dist/\n", "dist/"));
        assert!(ignores("/dist\n", "dist/"));
        assert!(ignores("target\n.coldluau\n", ".coldluau/"));
        assert!(!ignores("distros/\n", "dist/"));
        assert!(!ignores("", "dist/"));
    }
}
