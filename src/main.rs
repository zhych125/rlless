//! rlless - High-Performance Terminal Log Viewer
//!
//! A fast, memory-efficient terminal log viewer designed to handle extremely large files.

use anyhow::Result;
use clap::{Arg, Command};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging for development
    env_logger::init();

    // Parse command-line arguments
    let matches = Command::new("rlless")
        .version(rlless::VERSION)
        .about("A high-performance terminal log viewer for large files")
        .long_about(
            "rlless is a terminal-based log viewer that can handle extremely large files \
             (40GB+) with SIMD-optimized search and memory-efficient streaming.",
        )
        .arg(
            Arg::new("file")
                .help("Path to the log file to view")
                .required(true)
                .index(1),
        )
        .get_matches();

    // Get the file path argument
    let file_path = PathBuf::from(
        matches
            .get_one::<String>("file")
            .expect("file argument is required"),
    );

    // Validate file exists
    if !file_path.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }

    if !file_path.is_file() {
        anyhow::bail!("Path is not a regular file: {}", file_path.display());
    }

    // Initialize the Application and start the interactive event loop
    use rlless::ui::TerminalUI;
    use rlless::Application;

    let ui_renderer = Box::new(TerminalUI::new()?);
    let mut app = Application::new(&file_path, ui_renderer).await?;

    app.run().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_version_constant() {
        // Ensure version is accessible
        assert!(!rlless::VERSION.is_empty());
    }
}
